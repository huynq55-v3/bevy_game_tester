use bevy::{ecs::schedule::ExecutorKind, prelude::*};
use semantic_rl_fuzzer::{
    FuzzConfig, FuzzCorpus, FuzzEngine, FuzzEnvironment, OracleStatus, StepResult, TruthOracle,
    burn_helpers::{ActionTranslator, create_cpu_agent},
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

const VALUE_POOL: [i32; 6] = [10, -10, 42, 99, 0, 9999];

// THÊM ĐẠI DIỆN CLONE ĐỂ THỎA MÃN ĐA LUỒNG
#[derive(Clone)]
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
    pub last_step_crashed: bool,
}

impl BevyEnv {
    pub fn new() -> Self {
        let world = World::new();
        let mut schedule = Schedule::default();

        // ÉP BEVY CHẠY ĐƠN LUỒNG BÊN TRONG (Để nhường quyền đa luồng cho Rayon bên ngoài)
        schedule.set_executor_kind(ExecutorKind::SingleThreaded);
        schedule.add_systems((apply_poison_system, check_death_system).chain());

        Self {
            world,
            schedule,
            alive_entities: Vec::new(),
            current_episode: 0,
            last_step_crashed: false,
        }
    }

    fn sync_alive_entities(&mut self) {
        self.alive_entities.clear();
        let mut query = self.world.query::<Entity>();
        for entity in query.iter(&self.world) {
            self.alive_entities.push(entity);
        }
    }
}

// 🌟 TRICK QUAN TRỌNG: Tự implement Clone cho BevyEnv
// Vì Bevy World không thể Clone, ta sẽ tạo một World mới hoàn toàn cho mỗi luồng.
impl Clone for BevyEnv {
    fn clone(&self) -> Self {
        BevyEnv::new()
    }
}

impl FuzzEnvironment for BevyEnv {
    type State = Vec<f32>;
    type Action = BevyAction;

    fn get_state(&self) -> Self::State {
        let mut total_hp = 0;
        let mut total_poison = 0;

        // Chỉ thống kê các chỉ số vĩ mô của thế giới
        for &entity in &self.alive_entities {
            let hp = self.world.get::<Health>(entity).map(|h| h.0).unwrap_or(0);
            let p = self.world.get::<Poison>(entity).map(|p| p.0).unwrap_or(0);

            total_hp += hp;
            total_poison += p;
        }

        // Trả về đúng 4 chiều không gian mù lòa.
        // AI phải tự tìm ra ý nghĩa của việc thay đổi các con số này.
        vec![
            (self.alive_entities.len() as f32) / 100.0,
            (self.world.archetypes().len() as f32) / 10.0,
            (total_hp as f32) / 1000.0,
            (total_poison as f32) / 1000.0,
        ]
    }

    fn get_action_mask(&self) -> Vec<Vec<bool>> {
        // Head 0: Loại hành động (Kích thước 5)
        let mut action_type_mask = vec![true; 5];
        if self.alive_entities.is_empty() {
            action_type_mask[1..4].fill(false); // Cấm Despawn, Health, Poison
        }

        // Head 1: Entity Index (Kích thước 5)
        let mut entity_idx_mask = vec![false; 5];
        let num_alive = self.alive_entities.len().min(5); // Tối đa 5 khe

        if num_alive > 0 {
            // Chỉ mở khóa các khe ID tương ứng với số lượng quái vật đang sống
            entity_idx_mask[0..num_alive].fill(true);
        } else {
            // MẸO: Nếu không có quái nào, vẫn phải mở đại 1 khe để Softmax không bị lỗi NaN.
            // Đằng nào Head 0 cũng đã khóa lệnh thao tác rồi nên khe này vô hại.
            entity_idx_mask[0] = true;
        }

        // Trả về: [Mask Lệnh, Mask Tham Số ID, Không Mask Value]
        vec![action_type_mask, entity_idx_mask, vec![]]
    }

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

#[derive(Clone)]
pub struct BevyOracle;

impl TruthOracle<BevyEnv> for BevyOracle {
    fn judge(&self, env: &mut BevyEnv, is_invalid: bool) -> OracleStatus {
        if env.last_step_crashed {
            return OracleStatus::Violated;
        }
        if is_invalid {
            return OracleStatus::Invalid;
        }

        // ÉP CHẾ ĐỘ PURE EXPLORATION: Oracle không cho điểm trung gian.
        // Mạng Neural phải tự tìm động lực từ sự "Ngạc nhiên" (ICM).
        OracleStatus::Hold { reward: 0.0 }
    }
}

// ==========================================
// 5. MAIN FUNCTION: CONNECTING EVERYTHING
// ==========================================

fn main() {
    println!("🚀 Starting Bevy Game Engine Fuzzer (Parallel CPU Actor Mode)...");

    let head_sizes = vec![5, 5, 6];
    let agent = create_cpu_agent(4, 512, &head_sizes, 0.001, BevyTranslator);

    // Sử dụng Cấu hình mới, sạch sẽ và gom gọn
    let config = FuzzConfig {
        num_envs: 512,
        max_steps_per_episode: 100,
        total_iterations: 50_000,
        log_interval: 10,
    };

    let base_env = BevyEnv::new(); // Môi trường chuẩn để clone ra các luồng
    let oracle = BevyOracle;
    let corpus = FuzzCorpus::new(); // Dùng Corpus để giữ bug artifacts

    let mut engine = FuzzEngine {
        base_env,
        agent,
        oracle,
        corpus,
        config,
    };

    println!(
        "🔥 Starting Parallel Fuzzing with {} environments...",
        engine.config.num_envs
    );

    // 🌟 TRUYỀN CALLBACK ĐỂ TỰ BẮT VÀ IN LOG THEO NGÔN NGỮ CỦA GAME
    engine.run_fuzzing(|iteration, rollouts| {
        let mut action_counts = [0; 5];
        let mut total_actions = 0;

        // Quét toàn bộ hành vi của AI trong lô vừa rồi
        for traj in rollouts {
            for action in &traj.actions {
                total_actions += 1;
                match action {
                    BevyAction::SpawnEmpty => action_counts[0] += 1,
                    BevyAction::Despawn(_) => action_counts[1] += 1,
                    BevyAction::AddHealth(_, _) => action_counts[2] += 1,
                    BevyAction::AddPoison(_, _) => action_counts[3] += 1,
                    BevyAction::RunSchedule => action_counts[4] += 1,
                }
            }
        }

        if total_actions > 0 {
            let p_spawn = (action_counts[0] as f32 / total_actions as f32) * 100.0;
            let p_despawn = (action_counts[1] as f32 / total_actions as f32) * 100.0;
            let p_health = (action_counts[2] as f32 / total_actions as f32) * 100.0;
            let p_poison = (action_counts[3] as f32 / total_actions as f32) * 100.0;
            let p_run = (action_counts[4] as f32 / total_actions as f32) * 100.0;

            println!(
                "   🎮 [Bevy Fuzzer] Spawn ({:.1}%) | Despawn ({:.1}%) | +Health ({:.1}%) | +Poison ({:.1}%) | RunSched ({:.1}%)",
                p_spawn, p_despawn, p_health, p_poison, p_run
            );
        }
    });

    println!("✅ Fuzzing process completed.");
}
