use rand::seq::SliceRandom;
use semantic_rl_fuzzer::{
    agent::{ActionTranslator, create_agent},
    core::{
        FuzzConfig, FuzzCorpus, FuzzEngine, FuzzEnvironment, OracleStatus, StepResult, TruthOracle,
    },
    models::ModelArchitecture,
};

const VALUE_POOL: [i32; 3] = [42, 99, 9999];

#[derive(Clone, Debug)]
pub enum AlchemyAction {
    Intake(i32),
    Transfer(usize, usize),
    Catalyze(usize),
    Transmute,
}

#[derive(Clone)]
pub struct AlchemyTranslator;

impl ActionTranslator for AlchemyTranslator {
    type TargetAction = AlchemyAction;
    fn translate(&self, head_outputs: &[usize]) -> Self::TargetAction {
        let act_type = head_outputs[0] % 4;
        match act_type {
            0 => AlchemyAction::Intake(VALUE_POOL[head_outputs[1] % 3]),
            1 => AlchemyAction::Transfer(head_outputs[1] % 3, head_outputs[2] % 3),
            2 => AlchemyAction::Catalyze(head_outputs[1] % 3),
            _ => AlchemyAction::Transmute,
        }
    }
}

#[derive(Clone)]
pub struct AlchemyEnv {
    pub flasks: [i32; 3],
    pub permutation: [usize; 3], // 🌟 MÃ TRẬN HOÁN VỊ: Slot 0 chứa Bình nào?
    pub last_step_crashed: bool,
}

impl AlchemyEnv {
    pub fn new() -> Self {
        let mut env = Self {
            flasks: [0; 3],
            permutation: [0, 1, 2],
            last_step_crashed: false,
        };
        env.shuffle_slots();
        env
    }

    fn shuffle_slots(&mut self) {
        let mut rng = rand::rng();
        self.permutation.shuffle(&mut rng); // Xáo trộn vị trí các bình trong State
    }
}

impl FuzzEnvironment for AlchemyEnv {
    type State = Vec<f32>;
    type Action = AlchemyAction;

    fn get_state(&self) -> Self::State {
        let mut state = vec![0.0; 18];
        for i in 0..3 {
            // 🌟 MA GIÁO: Bình i sẽ được hiển thị tại vị trí permutation[i]
            let flask_val = self.flasks[i];
            let slot_idx = self.permutation[i];

            let mut h = (flask_val.abs() as u32).wrapping_mul(2654435761);
            h ^= h >> 16;
            for bit in 0..6 {
                state[slot_idx * 6 + bit] = if (h >> bit) & 1 == 1 { 1.0 } else { 0.0 };
            }
        }
        state
    }

    fn get_action_mask(&self) -> Vec<Vec<bool>> {
        vec![vec![true; 4], vec![true; 6], vec![true; 6]]
    }

    fn step(&mut self, action: &Self::Action) -> StepResult<Self::State> {
        let mut failed = false;
        match action {
            AlchemyAction::Intake(val) => {
                if self.flasks[0] == 0 {
                    self.flasks[0] = *val;
                } else {
                    failed = true;
                }
            }
            AlchemyAction::Transfer(src, dst) => {
                if *src != *dst && self.flasks[*src] != 0 && self.flasks[*dst] == 0 {
                    self.flasks[*dst] = self.flasks[*src];
                    self.flasks[*src] = 0;
                } else {
                    failed = true;
                }
            }
            AlchemyAction::Catalyze(target) => {
                let neighbor = (*target + 1) % 3;
                if self.flasks[neighbor] == 0
                    && (self.flasks[*target] == 42 || self.flasks[*target] == 99)
                {
                    self.flasks[*target] = if self.flasks[*target] == 42 { 1 } else { 2 };
                } else {
                    failed = true;
                }
            }
            AlchemyAction::Transmute => {
                if self.flasks[0] == 9999 && self.flasks[1] == 2 && self.flasks[2] == 1 {
                    self.last_step_crashed = true;
                } else {
                    failed = true;
                }
            }
        }

        if failed {
            self.reset(); // Mỗi lần fail là xáo lại vị trí
        }
        StepResult {
            next_state: self.get_state(),
            is_invalid: false,
        }
    }

    fn reset(&mut self) {
        self.flasks = [0; 3];
        self.last_step_crashed = false;
        self.shuffle_slots(); // 🌟 XÁO BÀI!
    }

    fn hash_state(state: &Self::State) -> u64 {
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for &val in state {
            hasher.write_u32(val.to_bits());
        }
        hasher.finish()
    }
}

pub struct AlchemyOracle;
impl TruthOracle<AlchemyEnv> for AlchemyOracle {
    fn judge(&self, env: &mut AlchemyEnv, _is_invalid: bool) -> OracleStatus {
        if env.last_step_crashed {
            return OracleStatus::Violated;
        }
        OracleStatus::Hold { reward: 0.0 }
    }
}

fn main() {
    println!("👺 KHỞI ĐỘNG MARAUDER V2 - MA GIÁO LỤC ĐỒ TRẬN...");

    let head_sizes = vec![4, 6, 6];
    let agent = create_agent(
        ModelArchitecture::Transformer,
        18,
        64,
        &head_sizes,
        0.0007,
        AlchemyTranslator,
        0.5, // 🌟 TĂNG MẠNH: Ép không được học vẹt
        0.2, // 🌟 TĂNG NHẸ: Bơm thêm tính liều mạng
        0.1,
        1024,
        10,
    );

    let mut engine = FuzzEngine {
        base_env: AlchemyEnv::new(),
        agent,
        oracle: AlchemyOracle,
        corpus: FuzzCorpus::new(),
        config: FuzzConfig {
            num_envs: 1024,
            max_steps_per_episode: 30,
            total_iterations: 10000,
            log_interval: 10,
        },
    };

    let mut global_max_reward = 0.0;

    engine.run_fuzzing(move |iteration, rollouts| {
        let mut crash_count = 0;
        let mut winning_actions = None;

        for traj in rollouts {
            // 🕵️ TRÍCH XUẤT HỒ SƠ MẬT: Bắt quả tang Seed xịn nhất
            if traj.reward > global_max_reward {
                global_max_reward = traj.reward;
                println!("==================================================");
                println!("🏆 KỶ LỤC MỚI TẠI ITERATION {}!", iteration);
                println!("⭐ Điểm Coverage tích lũy: {}", global_max_reward);
                println!(
                    "📜 Chuỗi hành động để đạt được ({} bước):",
                    traj.actions.len()
                );
                println!("{:#?}", traj.actions);
                println!("==================================================");
            }

            if traj.is_crash {
                crash_count += 1;
                winning_actions = Some(traj.actions.clone());
            }
        }

        if crash_count > 0 {
            println!("==================================================");
            println!(
                "💥 BÙM! CORE MELTDOWN! TÌM THẤY MÃ KÍCH NỔ TẠI ITERATION {}!",
                iteration
            );
            if let Some(actions) = winning_actions {
                println!(
                    "🏆 MÃ GIẢ KIM HOÀN HẢO (BẤM ĐÚNG TRANSMUTE):\n{:#?}",
                    actions
                );
            }
            println!("==================================================");
            std::process::exit(0);
        }
    });
}
