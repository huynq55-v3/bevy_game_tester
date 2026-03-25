use bevy::{ecs::schedule::ExecutorKind, prelude::*};
use semantic_rl_fuzzer::{
    FuzzConfig, FuzzCorpus, FuzzEngine, FuzzEnvironment, OracleStatus, StepResult, TruthOracle,
    burn_helpers::{ActionTranslator, create_cpu_agent},
};
use std::{panic, collections::VecDeque};

// ==========================================
// 1. BEVY GAME LOGIC (OUR VICTIM)
// ==========================================

#[derive(Component)]
struct Health(i32);

#[derive(Component)]
struct Poison(i32);

/// System 1: Độc phân rã theo thời gian và trừ máu (Có Tracking)
fn apply_poison_system(mut query: Query<(Entity, &mut Health, &mut Poison)>) {
    for (entity, mut health, mut poison) in query.iter_mut() {
        let old_health = health.0;
        let old_poison = poison.0;

        // 1. Logic ẩn: Độc tố bay hơi (giảm 10) sau mỗi chu kỳ
        if poison.0 > 0 {
            poison.0 -= 10;
        }

        // 2. Trừ máu bằng ĐỘC TỐ HIỆN TẠI (Sau khi đã bay hơi)
        health.0 -= poison.0;

        // --- 🎥 CAMERA THEO DÕI TIẾN ĐỘ CỦA AI ---
        // Chỉ báo cáo khi AI đã múa thành công cho Độc giảm về đúng 69.
        // Điều này chứng minh hàm phân rã thời gian hoạt động hoàn hảo!
        if poison.0 == 69 {
            println!(
                "⏱️ [TRACE] Entity {:?} | Độc = 69! | Trừ máu: {} -> {} (Do chịu {} dame)",
                entity, old_health, health.0, poison.0
            );
        }

        // 🚨 THE TEMPORAL BUG (LỖI THỜI GIAN) 🚨
        if health.0 == 30 && poison.0 == 69 {
            panic!(
                "☢️ FATAL TIME-BOMB BUG: Core Meltdown! Sequential logic triggered at Health=30, Poison=69!"
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

const VALUE_POOL: [i32; 7] = [10, -10, 42, 99, 141, 0, 9999];

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
    // 🧠 BỘ NHỚ NGẮN HẠN (FRAME STACKING)
    pub frame_buffer: VecDeque<Vec<f32>>,
}

impl BevyEnv {
    pub fn new() -> Self {
        let world = World::new();
        let mut schedule = Schedule::default();

        // ÉP BEVY CHẠY ĐƠN LUỒNG BÊN TRONG (Để nhường quyền đa luồng cho Rayon bên ngoài)
        schedule.set_executor_kind(ExecutorKind::SingleThreaded);
        schedule.add_systems((apply_poison_system, check_death_system).chain());

        let mut env = Self {
            world,
            schedule,
            alive_entities: Vec::new(),
            current_episode: 0,
            last_step_crashed: false,
            frame_buffer: VecDeque::with_capacity(4), // Nhớ 4 nhịp gần nhất!
        };

        // Khởi tạo buffer bằng các frame rỗng ban đầu (phải gọi sau khi struct đã được tạo)
        for _ in 0..4 {
            env.frame_buffer.push_back(env.get_single_frame_state());
        }
        env
    }

    /// 👁️ HYBRID STATE (Mắt thần nhìn cả Cờ lẫn Số thực)
    fn get_single_frame_state(&self) -> Vec<f32> {
        // 1. Tỷ lệ quái vật còn sống (1 chiều)
        let mut frame = vec![(self.alive_entities.len() as f32) / 5.0];

        // 2. Histogram (Bảng điểm danh Cờ) - Rất tốt cho Logic (14 chiều nếu VALUE_POOL.len()==7)
        let mut health_flags = vec![0.0; VALUE_POOL.len()];
        let mut poison_flags = vec![0.0; VALUE_POOL.len()];

        // 3. Normalized Raw Values - Rất tốt cho Sinh tồn và Nhịp điệu thời gian (2 chiều)
        let mut total_hp = 0.0;
        let mut total_poison = 0.0;

        for &entity in &self.alive_entities {
            let hp = self.world.get::<Health>(entity).map(|h| h.0).unwrap_or(0);
            let p = self.world.get::<Poison>(entity).map(|p| p.0).unwrap_or(0);

            total_hp += hp as f32;
            total_poison += p as f32;

            for (i, &val) in VALUE_POOL.iter().enumerate() {
                if hp == val { health_flags[i] = 1.0; }
                if p == val { poison_flags[i] = 1.0; }
            }
        }

        // TÍNH TOÁN THEO RANGE (Tuyến tính, Min-Max Scaling)
        // Tổng lớn nhất có thể có: 9999 * 5 = 49,995 (Làm tròn 50,000)
        let max_abs_value = 50000.0;

        // Scale tuyến tính: Giá trị sẽ luôn nằm gọn trong khoảng [-1.0, 1.0]
        frame.push(total_hp / max_abs_value);
        frame.push(total_poison / max_abs_value);

        // Gắn cờ vào
        frame.extend(health_flags);
        frame.extend(poison_flags);

        frame // Kích thước 1 frame = 1 + 2 + (7 * 2) = 17 chiều.
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
        // 🥞 TRẢ VỀ STATE XẾP CHỒNG (FRAME STACKING)
        // Ghép 4 frames gần nhất thành một mảng 1D
        let mut stacked_state = Vec::with_capacity(17 * 4);
        for frame in self.frame_buffer.iter() {
            stacked_state.extend_from_slice(frame);
        }
        stacked_state
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

        // Cập nhật trí nhớ ngắn hạn!
        self.frame_buffer.pop_front();
        self.frame_buffer.push_back(self.get_single_frame_state());

        StepResult {
            next_state: self.get_state(), // Trả về toàn bộ lịch sử 4 bước
            is_invalid,
        }
    }

    fn reset(&mut self) {
        self.world.clear_entities();
        self.world.clear_trackers();
        self.alive_entities.clear();
        self.last_step_crashed = false;
        self.current_episode += 1;

        // Reset lại bộ nhớ
        self.frame_buffer.clear();
        for _ in 0..4 {
            self.frame_buffer.push_back(self.get_single_frame_state());
        }
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

    let head_sizes = vec![5, 5, 7]; // Head 2 matching VALUE_POOL.len()
    
    // 1 frame = 1(count) + 2(sums) + 7(h_flags) + 7(p_flags) = 17
    // Stack 4 frames = 17 * 4 = 68
    let input_size = (3 + VALUE_POOL.len() * 2) * 4;
    let agent = create_cpu_agent(input_size, 512, &head_sizes, 0.001, BevyTranslator);

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

#[test]
#[should_panic(expected = "FATAL TIME-BOMB BUG")]
fn test_temporal_bug_is_solvable() {
    println!("--- BẮT ĐẦU CHẠY TEST CASE CHỨNG MINH ---");
    let mut env = BevyEnv::new();

    // 1. Khởi tạo thực thể
    env.step(&BevyAction::SpawnEmpty);
    println!("Step 1: Sinh ra Entity 0");

    // 2. Bơm Độc 99. Nếu bơm máu ít, quái sẽ chết ngay ở nhịp RunSched đầu tiên do check_death_system!
    env.step(&BevyAction::AddPoison(0, 99));

    // 3. CHIẾN THUẬT SINH TỒN: Bơm 9999 máu để "gồng" qua những đợt sát thương đầu tiên
    env.step(&BevyAction::AddHealth(0, 9999));
    println!("Step 2 & 3: Bơm Độc(99) và Máu(9999) để sống sót.");

    // 4. Nhịp thời gian 1 (RunSched)
    // Độc: 99 -> 89. Máu: 9999 - 89 = 9910. (Quái vẫn sống > 0)
    env.step(&BevyAction::RunSchedule);
    println!("Step 4 (RunSched): Độc giảm còn 89, Máu còn 9910.");

    // 5. Nhịp thời gian 2 (RunSched)
    // Độc: 89 -> 79. Máu: 9910 - 79 = 9831. (Quái vẫn sống > 0)
    env.step(&BevyAction::RunSchedule);
    println!("Step 5 (RunSched): Độc giảm còn 79, Máu còn 9831.");

    // 6. NHÁT CHÉM QUYẾT ĐỊNH (GHI ĐÈ MÁU)
    // Ngay lúc Độc đang là 79, AI phải ném chính xác số 99 vào để GHI ĐÈ cái lượng máu 9831 kia.
    env.step(&BevyAction::AddHealth(0, 99));
    println!("Step 6: AI ném Health(99) vào! Lúc này Máu=99, Độc=79.");

    // 7. Nhịp thời gian 3 (KÍCH NỔ BUG)
    // Độc: 79 -> 69. Máu: 99 - 69 = 30.
    // ĐIỀU KIỆN MÁU=30 & ĐỘC=69 ĐÃ THỎA MÃN -> PANIC!
    println!("Step 7 (RunSched): Độc giảm còn 69, Máu bị trừ 69 (99 - 69 = 30). CHUẨN BỊ NỔ!");
    env.step(&BevyAction::RunSchedule);
}
