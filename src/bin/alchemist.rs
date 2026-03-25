use semantic_rl_fuzzer::{
    FuzzConfig, FuzzCorpus, FuzzEngine, FuzzEnvironment, OracleStatus, StepResult, TruthOracle,
    burn_helpers::{ActionTranslator, create_cpu_agent},
};

const VALUE_POOL: [i32; 6] = [10, -10, 42, 99, 0, 9999];

// ==========================================
// 1. ACTION & TRANSLATOR
// ==========================================
#[derive(Clone, Debug)]
pub enum AlchemyAction {
    Intake(i32),            // Bơm vào F0
    Transfer(usize, usize), // Đổ từ src sang dst
    Catalyze(usize),        // Kích hoạt biến đổi
    Transmute,              // Nút kích nổ
}

#[derive(Clone)]
pub struct AlchemyTranslator;

impl ActionTranslator for AlchemyTranslator {
    type TargetAction = AlchemyAction;

    fn translate(&self, head_outputs: &[usize]) -> Self::TargetAction {
        let act_type = head_outputs[0] % 4; // 4 loại hành động

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

// ==========================================
// 2. THE QUÁI THAI ENVIRONMENT
// ==========================================
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
            // 1. BIT VẬT LÝ: Có trống rỗng không? (1 chiều)
            // Đây là thông tin quan trọng nhất để né lỗi Catalyze.
            state.push(if f == 0 { 1.0 } else { 0.0 });

            // 2. BITS KÝ HIỆU: 4-bit Hash Fingerprint (4 chiều)
            // Dùng hằng số vàng của Knuth để băm số, phá vỡ hoàn toàn tính chất "to/nhỏ"
            let mut h = (f.abs() as u32).wrapping_mul(2654435761);
            h ^= h >> 16; // Trộn bit thêm một lần nữa

            // Lấy chính xác 4 bit cuối làm dấu vân tay
            for i in 0..4 {
                state.push(if (h >> i) & 1 == 1 { 1.0 } else { 0.0 });
            }
        }

        // Tổng cộng: (1 + 4) * 3 = 15 chiều.
        // Siêu gọn, AI sẽ học cực nhanh!
        state
    }

    fn get_action_mask(&self) -> Vec<Vec<bool>> {
        // Không dùng Mask để AI tự do vấp ngã
        vec![vec![true; 4], vec![true; 6], vec![true; 6]]
    }

    fn step(&mut self, action: &Self::Action) -> StepResult<Self::State> {
        match action {
            AlchemyAction::Intake(val) => {
                // LUẬT: Chỉ nạp khi bình F0 rỗng hoàn toàn
                if self.flasks[0] == 0 {
                    self.flasks[0] = *val;
                }
            }
            AlchemyAction::Transfer(src, dst) => {
                let s = *src;
                let d = *dst;
                // LUẬT: Chỉ đổ khi bình nguồn có đồ VÀ bình đích đang rỗng
                if s != d && self.flasks[s] != 0 && self.flasks[d] == 0 {
                    self.flasks[d] = self.flasks[s];
                    self.flasks[s] = 0;
                }
            }
            AlchemyAction::Catalyze(target) => {
                let t = *target;
                let neighbor = (t + 1) % 3;

                // THE TRAP: Phản ứng chỉ xảy ra nếu bình bên cạnh rỗng
                if self.flasks[neighbor] == 0 {
                    if self.flasks[t] == 42 {
                        self.flasks[t] = 1; // Vật chất 1
                    } else if self.flasks[t] == 99 {
                        self.flasks[t] = 2; // Vật chất 2
                    } else {
                        self.flasks[t] = 0; // Sai nguyên liệu -> Bay hơi
                    }
                } else {
                    // Nếu bình bên cạnh không rỗng -> Phản ứng thất bại, mất đồ
                    self.flasks[t] = 0;
                }
            }
            AlchemyAction::Transmute => {
                if self.flasks[0] == 9999 && self.flasks[1] == 2 && self.flasks[2] == 1 {
                    self.last_step_crashed = true;
                }
            }
        }

        StepResult {
            next_state: self.get_state(),
            is_invalid: false, // Vẫn giữ tự do, AI làm sai thì State không đổi (nhàm chán)
        }
    }

    fn reset(&mut self) {
        self.flasks = [0; 3];
        self.last_step_crashed = false;
    }
}

// ==========================================
// 3. ORACLE & MAIN
// ==========================================
#[derive(Clone)]
pub struct AlchemyOracle;

impl TruthOracle<AlchemyEnv> for AlchemyOracle {
    fn judge(&self, env: &mut AlchemyEnv, is_invalid: bool) -> OracleStatus {
        if env.last_step_crashed {
            return OracleStatus::Violated;
        }
        if is_invalid {
            return OracleStatus::Invalid;
        }
        OracleStatus::Hold { reward: 0.0 }
    }
}

fn main() {
    println!("🧪 Kích hoạt Lò Phản Ứng Giả Kim (Pure Rust)...");

    let head_sizes = vec![4, 6, 6];

    // Lưu ý: State dimension là 21 (không dùng Frame Stacking để xem bản năng gốc của nó)
    // Hoặc ông có thể tự wrap Frame Stacking vào AlchemyEnv như đã bàn!
    let agent = create_cpu_agent(
        15,
        512,
        &head_sizes,
        0.005,
        AlchemyTranslator,
        20.0,
        0.05,
        0.1,
    );

    let config = FuzzConfig {
        num_envs: 1024,             // Quất thẳng 1024 luồng vì Rust chạy quá nhẹ!
        max_steps_per_episode: 100, // Chuỗi ngắn thôi để nó reset nhanh
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
        let mut total_transmute = 0;

        let mut created_mat_1 = 0;
        let mut created_mat_2 = 0;
        let mut f0_is_9999 = 0;

        let mut best_score = 0;
        let mut best_trajectory_info = None;

        // 🌟 BẢO BỐI Ở ĐÂY: Biến lưu lại chuỗi hành động phá đảo
        let mut winning_actions = None;

        // --- HELPER: Hàm kiểm tra vân tay của một con số trong State ---
        let matches_val = |state: &[f32], flask_idx: usize, target: i32| -> bool {
            let offset = flask_idx * 5;
            if target == 0 {
                return state[offset] == 1.0;
            }
            if state[offset] == 1.0 {
                return false;
            }

            let mut h = (target.abs() as u32).wrapping_mul(2654435761);
            h ^= h >> 16;
            for i in 0..4 {
                let bit = if (h >> i) & 1 == 1 { 1.0 } else { 0.0 };
                if state[offset + 1 + i] != bit {
                    return false;
                }
            }
            true
        };

        for traj in rollouts {
            // 🌟 NẾU LÀ CHUỖI PHÁ ĐẢO (CRASH), LƯU NGAY LẠI!
            if traj.is_interesting && traj.reward >= 0.0 {
                crash_count += 1;
                winning_actions = Some(traj.actions.clone());
            }

            let mut traj_best_score = 0;
            let mut traj_status = String::new();

            for (step_idx, act) in traj.actions.iter().enumerate() {
                if let AlchemyAction::Transmute = act {
                    total_transmute += 1;
                }

                let state = &traj.states[step_idx + 1];

                let f0_9999 = matches_val(state, 0, 9999);
                let f1_mat2 = matches_val(state, 1, 2);
                let f2_mat1 = matches_val(state, 2, 1);

                if matches_val(state, 0, 1) || matches_val(state, 1, 1) || matches_val(state, 2, 1)
                {
                    created_mat_1 += 1;
                }
                if matches_val(state, 0, 2) || matches_val(state, 1, 2) || matches_val(state, 2, 2)
                {
                    created_mat_2 += 1;
                }
                if f0_9999 {
                    f0_is_9999 += 1;
                }

                let mut score = 0;
                if f0_9999 {
                    score += 1;
                }
                if f1_mat2 {
                    score += 1;
                }
                if f2_mat1 {
                    score += 1;
                }

                if score > traj_best_score {
                    traj_best_score = score;
                    traj_status = format!(
                        "F0={}, F1={}, F2={}",
                        if f0_9999 { "9999" } else { "?" },
                        if f1_mat2 { "MAT_2" } else { "?" },
                        if f2_mat1 { "MAT_1" } else { "?" }
                    );
                }
            }

            if traj_best_score > best_score {
                best_score = traj_best_score;
                best_trajectory_info = Some((traj_status, traj.actions.clone()));
            }
        }

        // 🌟 IN RA BẢNG VÀNG TRƯỚC KHI THOÁT
        if crash_count > 0 {
            println!("==================================================");
            println!(
                "💥 BÙM! CORE MELTDOWN! TÌM THẤY MÃ KÍCH NỔ TẠI ITERATION {}!",
                iteration
            );
            if let Some(actions) = winning_actions {
                println!("🏆 MÃ GIẢ KIM HOÀN HẢO:\n{:#?}", actions); // Dùng {:#?} để in format dọc cho dễ nhìn
            }
            println!("==================================================");
            std::process::exit(0);
        } else if iteration % config.log_interval == 0 {
            println!(
                "📊 [Iter {}] Thống kê Lò Phản Ứng (Binary Hash Mode):",
                iteration
            );
            println!("  ➤ F0 đạt mốc 9999: {} lần", f0_is_9999);
            println!("  ➤ Chế tạo thành công Vật chất 1: {} lần", created_mat_1);
            println!("  ➤ Chế tạo thành công Vật chất 2: {} lần", created_mat_2);
            println!(
                "  ➤ Bấm nút Transmute (Kíp nổ hụt): {} lần",
                total_transmute
            );

            if best_score >= 1 {
                if let Some((status, actions)) = &best_trajectory_info {
                    println!(
                        "  🔥 [TIẾN TRIỂN] AI ĐÃ CHẠM {}/3 ĐIỀU KIỆN! ({})",
                        best_score, status
                    );
                    println!("     Chuỗi mẫu: {:?}", actions);
                }
            }
            println!("--------------------------------------------------");
        }
    });
}
