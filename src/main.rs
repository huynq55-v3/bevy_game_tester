use bevy::prelude::*;
use semantic_rl_fuzzer::{
    FuzzEngine, FuzzEnvironment, HybridReplayBuffer, OracleStatus, StepResult, TruthOracle,
    burn_helpers::{ActionTranslator, create_gpu_agent},
};
use std::panic;

// ==========================================
// 1. BEVY GAME LOGIC (OUR VICTIM)
// ==========================================

#[derive(Component)]
struct Health(i32);

#[derive(Component)]
struct Poison(i32);

/// System 1: Subtract health if poisoned
fn apply_poison_system(mut query: Query<(&mut Health, &Poison)>) {
    for (mut health, poison) in query.iter_mut() {
        health.0 -= poison.0;

        // 🚨 HIDDEN BUG (HIDDEN LOGIC FLAW) 🚨
        if health.0 == 42 && poison.0 == 99 {
            panic!(
                "FATAL LOGIC BUG: Found integer overflow vulnerability when Health=42 and Poison=99!"
            );
        }
    }
}

/// System 2: Cleanup dead entities
fn check_death_system(mut commands: Commands, query: Query<(Entity, &Health)>) {
    for (entity, health) in query.iter() {
        if health.0 <= 0 {
            commands.entity(entity).despawn();
        }
    }
}

// ==========================================
// 2. DICTIONARY AND TRANSLATOR
// ==========================================

#[derive(Clone, Debug)]
pub enum BevyAction {
    SpawnEmpty,
    Despawn(usize),        // Parameter is the Entity Index in the alive list
    AddHealth(usize, i32), // Entity Index, Health Value
    AddPoison(usize, i32), // Entity Index, Poison Value
    RunSchedule,           // Force Bevy to run multithreaded systems
}

/// Pool of tricky numbers for AI to choose (instead of random floats)
const VALUE_POOL: [i32; 6] = [10, -10, 42, 99, 0, 9999];

pub struct BevyTranslator;

impl ActionTranslator for BevyTranslator {
    type TargetAction = BevyAction;

    fn translate(&self, head_outputs: &[usize]) -> Self::TargetAction {
        let action_type = head_outputs[0];
        let entity_idx = head_outputs[1];
        let value_idx = head_outputs[2] % VALUE_POOL.len();
        let value = VALUE_POOL[value_idx];

        match action_type {
            0 => BevyAction::SpawnEmpty,
            1 => BevyAction::Despawn(entity_idx),
            2 => BevyAction::AddHealth(entity_idx, value),
            3 => BevyAction::AddPoison(entity_idx, value),
            _ => BevyAction::RunSchedule,
        }
    }
}

// ==========================================
// 3. FUZZING ENVIRONMENT (Pure Execution Only)
// ==========================================

pub struct BevyEnv {
    pub world: World,
    pub schedule: Schedule,
    pub alive_entities: Vec<Entity>,
    pub current_episode: usize,
    /// Set by step() so the Oracle can check if a panic was caught.
    pub last_step_crashed: bool,
}

impl BevyEnv {
    pub fn new() -> Self {
        let world = World::new();
        let mut schedule = Schedule::default();
        schedule.add_systems((apply_poison_system, check_death_system).chain());

        Self {
            world,
            schedule,
            alive_entities: Vec::new(),
            current_episode: 0,
            last_step_crashed: false,
        }
    }

    /// Synchronize the alive entities list after Bevy runs systems
    fn sync_alive_entities(&mut self) {
        self.alive_entities.clear();
        let mut query = self.world.query::<Entity>();
        for entity in query.iter(&self.world) {
            self.alive_entities.push(entity);
        }
    }
}

impl FuzzEnvironment for BevyEnv {
    type State = Vec<f32>;
    type Action = BevyAction;

    fn get_state(&self) -> Self::State {
        let mut combo_count = 0;
        for &entity in &self.alive_entities {
            if self.world.get::<Health>(entity).is_some()
                && self.world.get::<Poison>(entity).is_some()
            {
                combo_count += 1;
            }
        }

        vec![
            self.alive_entities.len() as f32,
            self.world.archetypes().len() as f32,
            combo_count as f32,
        ]
    }

    fn get_action_mask(&self) -> Vec<bool> {
        let mut mask = vec![true; 5];
        if self.alive_entities.is_empty() {
            mask[1..4].fill(false);
        }
        mask
    }

    /// Pure execution — no oracle logic here. Just run the action and report facts.
    fn step(&mut self, action: &Self::Action) -> StepResult<Self::State> {
        let mut is_invalid = false;

        let catch_result = panic::catch_unwind(panic::AssertUnwindSafe(|| match action {
            BevyAction::SpawnEmpty => {
                let id = self.world.spawn_empty().id();
                self.alive_entities.push(id);
            }
            BevyAction::Despawn(idx) => {
                if let Some(&entity) = self.alive_entities.get(*idx) {
                    self.world.despawn(entity);
                    self.sync_alive_entities();
                } else {
                    is_invalid = true;
                }
            }
            BevyAction::AddHealth(idx, val) => {
                if let Some(&entity) = self.alive_entities.get(*idx) {
                    self.world.entity_mut(entity).insert(Health(*val));
                } else {
                    is_invalid = true;
                }
            }
            BevyAction::AddPoison(idx, val) => {
                if let Some(&entity) = self.alive_entities.get(*idx) {
                    self.world.entity_mut(entity).insert(Poison(*val));
                } else {
                    is_invalid = true;
                }
            }
            BevyAction::RunSchedule => {
                self.schedule.run(&mut self.world);
                self.sync_alive_entities();
            }
        }));

        // Store crash state for Oracle to inspect
        self.last_step_crashed = catch_result.is_err();

        StepResult {
            next_state: self.get_state(),
            is_invalid,
        }
    }

    fn reset(&mut self) {
        self.world.clear_entities();
        self.world.clear_trackers();
        self.alive_entities.clear();
        self.last_step_crashed = false;
        self.current_episode += 1;
    }
}

// ==========================================
// 4. TRUTH ORACLE (Semantic Bug Judge)
// ==========================================

pub struct BevyOracle;

impl TruthOracle<BevyEnv> for BevyOracle {
    fn judge(&self, env: &mut BevyEnv, is_invalid: bool) -> OracleStatus {
        // Check if the last step caused a crash (panic caught)
        if env.last_step_crashed {
            return OracleStatus::Violated;
        }
        if is_invalid {
            return OracleStatus::Invalid;
        }

        let mut total_reward = 0.1;
        let episode = env.current_episode;

        let mut query = env.world.query::<(Option<&Health>, Option<&Poison>)>();
        let results: Vec<_> = query.iter(&env.world).collect();

        for (h, p) in &results {
            if h.is_some() && p.is_some() {
                total_reward += 2.0;

                if episode % 100 == 0 {
                    println!(
                        "✨ [Episode {}] AI is maintaining Health+Poison Combo",
                        episode
                    );
                }

                let h_val = h.unwrap().0;
                let p_val = p.unwrap().0;

                if h_val == 42 {
                    total_reward += 10.0;
                }
                if p_val == 99 {
                    total_reward += 10.0;
                }
                if h_val == 42 && p_val == 99 {
                    total_reward += 50.0;
                    println!("🔥 [Episode {}] APPROACHING THE BUG! (42-99)", episode);
                }
            }
        }

        total_reward += (env.world.archetypes().len() as f32) * 0.05;
        OracleStatus::Hold {
            reward: total_reward,
        }
    }
}

// ==========================================
// 5. MAIN FUNCTION: CONNECTING EVERYTHING
// ==========================================

fn main() {
    println!("🚀 Starting Bevy Game Engine Fuzzer...");

    let head_sizes = vec![5, 5, 6];

    let agent = create_gpu_agent(3, 512, &head_sizes, 0.001, BevyTranslator);

    // Initialize an Environment Farm: 256 parallel Bevy worlds on CPU
    let num_envs = 256;
    let envs: Vec<BevyEnv> = (0..num_envs).map(|_| BevyEnv::new()).collect();

    let buffer = HybridReplayBuffer::new(5000);
    let oracle = BevyOracle;

    let mut engine = FuzzEngine {
        envs,
        agent,
        oracle,
        buffer,
        max_steps_per_episode: 50,
        batch_size: 256,
    };

    // Note: 50_000 cycles × 256 envs = 12.8 million episodes!
    println!("🔥 Starting Vectorized Fuzzing with {} environments...", num_envs);
    engine.run_fuzzing(50_000);
    println!("✅ Fuzzing process completed.");
}
