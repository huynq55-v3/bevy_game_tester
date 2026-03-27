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
            0 => {
                let val_idx = head_outputs[1] % VALUE_POOL.len();
                AlchemyAction::Intake(VALUE_POOL[val_idx])
            }
            1 => {
                let src = head_outputs[1] % 3;
                let dst = head_outputs[2] % 3;
                AlchemyAction::Transfer(src, dst)
            }
            2 => {
                let target = head_outputs[1] % 3;
                AlchemyAction::Catalyze(target)
            }
            _ => AlchemyAction::Transmute,
        }
    }
}

#[derive(Clone)]
pub struct AlchemyEnv {
    pub flasks: [i32; 3],
    pub last_step_crashed: bool,
}

impl AlchemyEnv {
    pub fn new() -> Self {
        Self {
            flasks: [0; 3],
            last_step_crashed: false,
        }
    }
}

impl FuzzEnvironment for AlchemyEnv {
    type State = Vec<f32>;
    type Action = AlchemyAction;

    fn get_state(&self) -> Self::State {
        let mut state = Vec::new();
        for &f in &self.flasks {
            let mut h = (f.abs() as u32).wrapping_mul(2654435761);
            h ^= h >> 16;
            for i in 0..6 {
                state.push(if (h >> i) & 1 == 1 { 1.0 } else { 0.0 });
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
                let (s, d) = (*src, *dst);
                if s != d && self.flasks[s] != 0 && self.flasks[d] == 0 {
                    self.flasks[d] = self.flasks[s];
                    self.flasks[s] = 0;
                } else {
                    failed = true;
                }
            }
            AlchemyAction::Catalyze(target) => {
                let t = *target;
                let neighbor = (t + 1) % 3;
                if self.flasks[neighbor] == 0 && (self.flasks[t] == 42 || self.flasks[t] == 99) {
                    self.flasks[t] = if self.flasks[t] == 42 { 1 } else { 2 };
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
            self.reset();
        }
        StepResult {
            next_state: self.get_state(),
            is_invalid: false,
        }
    }

    fn reset(&mut self) {
        self.flasks = [0; 3];
        self.last_step_crashed = false;
    }

    // 🌟 THÊM HÀM NÀY ĐỂ CORE CÓ THỂ ĐẾM ĐỘ PHỦ!
    fn hash_state(state: &Self::State) -> u64 {
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for &val in state {
            hasher.write_u32(val.to_bits());
        }
        hasher.finish()
    }
}

#[derive(Clone)]
pub struct AlchemyOracle;

impl TruthOracle<AlchemyEnv> for AlchemyOracle {
    fn judge(&self, env: &mut AlchemyEnv, is_invalid: bool) -> OracleStatus {
        if env.last_step_crashed {
            return OracleStatus::Violated; // CHẠM TỚI BUG LÀ THẮNG
        }
        if is_invalid {
            return OracleStatus::Invalid;
        }
        // KẾT LIỄU REWARD SHAPING (KẸO)! AI giờ tự sống bằng State Coverage.
        OracleStatus::Hold { reward: 0.0 }
    }
}

fn main() {
    println!("🧪 Kích hoạt Lò Phản Ứng Giả Kim (ĐỘ PHỦ STATE TỰ ĐỘNG)...");

    let head_sizes = vec![4, 6, 6];
    let agent = create_agent(
        ModelArchitecture::Mlp,
        18,
        128,
        &head_sizes,
        0.001,
        AlchemyTranslator,
        0.05, // Entropy coeff
        0.05, // Noise floor (Nhẹ nhàng thôi vì giờ có Coverage dẫn đường rồi)
        1024, // Batch size
    );

    let config = FuzzConfig {
        num_envs: 1024,
        max_steps_per_episode: 30, // Đủ dài để nó ráp 15 bước
        total_iterations: 10_000,
        log_interval: 10,
    };

    let mut engine = FuzzEngine {
        base_env: AlchemyEnv::new(),
        agent,
        oracle: AlchemyOracle,
        corpus: FuzzCorpus::new(),
        config: config.clone(),
    };

    engine.run_fuzzing(|iteration, rollouts| {
        let mut crash_count = 0;
        let mut winning_actions = None;

        for traj in rollouts {
            if traj.is_interesting && traj.reward >= 0.0 {
                // Check xem nếu cuối hành trình last_step_crashed = true (tức là nổ lò)
                if traj
                    .actions
                    .last()
                    .map(|a| matches!(a, AlchemyAction::Transmute))
                    .unwrap_or(false)
                {
                    crash_count += 1;
                    winning_actions = Some(traj.actions.clone());
                }
            }
        }

        if crash_count > 0 {
            println!("==================================================");
            println!(
                "💥 BÙM! CORE MELTDOWN! TÌM THẤY MÃ KÍCH NỔ TẠI ITERATION {}!",
                iteration
            );
            if let Some(actions) = winning_actions {
                println!("🏆 MÃ GIẢ KIM HOÀN HẢO:\n{:#?}", actions);
            }
            println!("==================================================");
            std::process::exit(0);
        }
    });
}
