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
            // state.push(if f == 0 { 1.0 } else { 0.0 });

            // 2. BITS KÝ HIỆU: 4-bit Hash Fingerprint (4 chiều)
            // Dùng hằng số vàng của Knuth để băm số, phá vỡ hoàn toàn tính chất "to/nhỏ"
            let mut h = (f.abs() as u32).wrapping_mul(2654435761);
            h ^= h >> 16; // Trộn bit thêm một lần nữa

            // (1 bit Is_Zero + 6 bit Hash) * 3 bình = 21 chiều
            for i in 0..6 {
                state.push(if (h >> i) & 1 == 1 { 1.0 } else { 0.0 });
            }
        }

        // Tổng cộng: 6 * 3 = 18 chiều.
        // Siêu gọn, AI sẽ học cực nhanh!
        state
    }

    fn get_action_mask(&self) -> Vec<Vec<bool>> {
        // Không dùng Mask để AI tự do vấp ngã
        vec![vec![true; 4], vec![true; 6], vec![true; 6]]
    }

    fn step(&mut self, action: &Self::Action) -> StepResult<Self::State> {
        let mut failed = false;

        match action {
            AlchemyAction::Intake(val) => {
                if self.flasks[0] == 0 {
                    self.flasks[0] = *val;
                } else {
                    failed = true; // Nạp đè
                }
            }
            AlchemyAction::Transfer(src, dst) => {
                let s = *src;
                let d = *dst;
                if s != d && self.flasks[s] != 0 && self.flasks[d] == 0 {
                    self.flasks[d] = self.flasks[s];
                    self.flasks[s] = 0;
                } else {
                    failed = true; // Đổ rác hoặc đổ nhầm bình
                }
            }
            AlchemyAction::Catalyze(target) => {
                let t = *target;
                let neighbor = (t + 1) % 3;

                // Phản ứng chỉ xảy ra nếu bình bên cạnh rỗng VÀ đúng nguyên liệu
                if self.flasks[neighbor] == 0 && (self.flasks[t] == 42 || self.flasks[t] == 99) {
                    if self.flasks[t] == 42 {
                        self.flasks[t] = 1;
                    } else {
                        self.flasks[t] = 2;
                    }
                } else {
                    failed = true; // Vi phạm luật bẫy hoặc sai hóa chất
                }
            }
            AlchemyAction::Transmute => {
                if self.flasks[0] == 9999 && self.flasks[1] == 2 && self.flasks[2] == 1 {
                    self.last_step_crashed = true;
                } else {
                    failed = true; // Kích nổ non -> Sập lò
                }
            }
        }

        // NẾU THẤT BẠI: Reset toàn bộ công sức của Episode này
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

    // Nâng lên 21 chiều (6-bit Hash cho mỗi bình)
    let agent = create_cpu_agent(
        18,
        1024,
        &head_sizes,
        0.005,
        AlchemyTranslator,
        30.0, // Tăng intrinsic_weight (eta) lên để tò mò mạnh hơn
        0.2,  // Tăng entropy_coeff (beta) lên để ép AI phải "phân vân"
        0.1,  // Noise floor giữ mức 0.1 để tay luôn "rung"
        0.001,
        1024,
        2_000_000,
    );

    let config = FuzzConfig {
        num_envs: 1024,
        max_steps_per_episode: 64, // Để 64 cho nó vừa làm vừa chơi, dễ nổ hơn 32
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
            // Bây giờ mỗi bình chỉ có 6 chiều (chỉ có Hash), không còn bit Check Trống
            let offset = flask_idx * 6;

            // Tính toán Fingerprint (Hash) của cái "target" mình đang muốn check
            // Kể cả target == 0 thì mình cũng băm nó ra để so sánh bit-to-bit
            let mut h = (target.abs() as u32).wrapping_mul(2654435761);
            h ^= h >> 16;

            // So khớp 6 bit Hash trực tiếp trong state
            for i in 0..6 {
                let bit = if (h >> i) & 1 == 1 { 1.0 } else { 0.0 };
                if state[offset + i] != bit {
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
