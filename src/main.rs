use bevy::prelude::*;
use semantic_rl_fuzzer::{
    FuzzEngine,
    FuzzEnvironment,
    HybridReplayBuffer,
    OracleStatus,
    StepResult,
    burn_helpers::{ActionTranslator, create_gpu_agent}, // Import Nhà máy
};
use std::panic;

// ==========================================
// 1. BEVY GAME LOGIC (NẠN NHÂN CỦA CHÚNG TA)
// ==========================================

#[derive(Component)]
struct Health(i32);

#[derive(Component)]
struct Poison(i32);

/// System 1: Trừ máu nếu bị trúng độc
fn apply_poison_system(mut query: Query<(&mut Health, &Poison)>) {
    for (mut health, poison) in query.iter_mut() {
        health.0 -= poison.0;

        // 🚨 HIDDEN BUG (LỖ HỔNG LOGIC ẨN) 🚨
        if health.0 == 42 && poison.0 == 99 {
            panic!("FATAL LOGIC BUG: Tìm thấy lỗ hổng tràn số khi Health=42 và Poison=99!");
        }
    }
}

/// System 2: Dọn dẹp xác chết
fn check_death_system(mut commands: Commands, query: Query<(Entity, &Health)>) {
    for (entity, health) in query.iter() {
        if health.0 <= 0 {
            commands.entity(entity).despawn();
        }
    }
}

// ==========================================
// 2. TỪ ĐIỂN VÀ TRÌNH PHIÊN DỊCH (TRANSLATOR)
// ==========================================

#[derive(Clone, Debug)]
pub enum BevyAction {
    SpawnEmpty,
    Despawn(usize),        // Tham số là Index của Entity trong danh sách alive
    AddHealth(usize, i32), // Index Entity, Giá trị Máu
    AddPoison(usize, i32), // Index Entity, Giá trị Độc
    RunSchedule,           // Ép Bevy chạy các System đa luồng
}

/// Bể chứa các con số hiểm hóc để AI chọn (Thay vì sinh số float ngẫu nhiên)
const VALUE_POOL: [i32; 6] = [10, -10, 42, 99, 0, 9999];

pub struct BevyTranslator;

impl ActionTranslator for BevyTranslator {
    type TargetAction = BevyAction;

    fn translate(&self, head_outputs: &[usize]) -> Self::TargetAction {
        let action_type = head_outputs[0];
        let entity_idx = head_outputs[1];
        let value_idx = head_outputs[2] % VALUE_POOL.len(); // Tránh out of bounds
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
// 3. MÔI TRƯỜNG FUZZING (THE ENVIRONMENT)
// ==========================================

pub struct BevyEnv {
    pub world: World,
    pub schedule: Schedule,
    pub alive_entities: Vec<Entity>, // Môi trường tự theo dõi các ID hợp lệ
}

impl BevyEnv {
    pub fn new() -> Self {
        let world = World::new();
        let mut schedule = Schedule::default();
        // Xếp lịch chạy System theo chuỗi
        schedule.add_systems((apply_poison_system, check_death_system).chain());

        Self {
            world,
            schedule,
            alive_entities: Vec::new(),
        }
    }

    /// Đồng bộ lại danh sách Entity sau khi Bevy chạy các System (vì System có thể despawn)
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

    /// Mắt của AI: Nhìn vào "Sức khỏe" của Bevy
    fn get_state(&self) -> Self::State {
        vec![
            self.alive_entities.len() as f32,     // Số lượng quái vật
            self.world.archetypes().len() as f32, // Độ phân mảnh bộ nhớ
        ]
    }

    /// KỸ THUẬT CHE MẶT NẠ (ACTION MASKING)
    fn get_action_mask(&self) -> Vec<bool> {
        let mut mask = vec![true; 5]; // Có 5 hành động ở Head 0

        // CẤM AI gọi hàm Despawn, AddHealth, AddPoison nếu trên map chưa có quái vật nào!
        if self.alive_entities.is_empty() {
            mask[1] = false; // Despawn
            mask[2] = false; // AddHealth
            mask[3] = false; // AddPoison
        }
        mask
    }

    fn step(&mut self, action: &Self::Action) -> StepResult<Self::State> {
        let mut is_invalid = false;

        // Bọc trong catch_unwind để AI không bị văng ra ngoài khi Bevy gặp lỗi Panic
        let catch_result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            match action {
                BevyAction::SpawnEmpty => {
                    let id = self.world.spawn_empty().id();
                    self.alive_entities.push(id);
                }
                BevyAction::Despawn(idx) => {
                    if let Some(&entity) = self.alive_entities.get(*idx) {
                        self.world.despawn(entity);
                        self.sync_alive_entities();
                    } else {
                        is_invalid = true; // AI đẻ ra Index tào lao -> Báo lỗi rác
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
                    self.sync_alive_entities(); // System dọn dẹp xác chết, phải đồng bộ lại
                }
            }
        }));

        // THẨM PHÁN CHẤM ĐIỂM (THE TRUTH ORACLE)
        if catch_result.is_err() {
            // BEVY ĐÃ PANIC (Bị sập)! TÌM THẤY BUG LOGIC!
            return StepResult {
                next_state: self.get_state(),
                reward: 100.0, // Jackpot!
                is_violated: true,
                is_invalid: false,
            };
        }

        if is_invalid {
            // Lệnh sai tham số, cấm không cho điểm
            return StepResult {
                next_state: self.get_state(),
                reward: -1.0,
                is_violated: false,
                is_invalid: true,
            };
        }

        // Vượt qua màng lọc, chạy tốt -> Thưởng một chút để khuyến khích
        StepResult {
            next_state: self.get_state(),
            reward: 0.1,
            is_violated: false,
            is_invalid: false,
        }
    }

    fn reset(&mut self) {
        self.world.clear_entities();
        self.world.clear_trackers();

        self.alive_entities.clear();
    }
}

// ==========================================
// 4. HÀM MAIN: KẾT NỐI MỌI THỨ
// ==========================================

fn main() {
    println!("🚀 Khởi động Bevy Game Engine Fuzzer...");

    let head_sizes = vec![5, 100, 6];

    // SỰ THANH LỊCH LÀ ĐÂY:
    // Gọi Nhà máy của Lib để đẻ ra Agent. Crate Bin không cần biết bên trong có Tensor.
    let agent = create_gpu_agent(
        2,              // Số lượng thông số State (Input size)
        512,            // Số nơ-ron lớp ẩn (Hidden size)
        &head_sizes,    // Cấu trúc 3 cái Đầu
        0.001,          // Tốc độ học (Learning rate)
        BevyTranslator, // Đưa Trình phiên dịch của chúng ta cho AI
    );

    let env = BevyEnv::new();
    let buffer = HybridReplayBuffer::new(5000);

    let mut engine = FuzzEngine {
        env,
        agent,
        buffer,
        // Ép AI bắn 50 lệnh liên tiếp mới hết 1 Hồi
        max_steps_per_episode: 50, 
        // Ép AI phải gom đủ 256 Hồi mới được gửi sang GPU 1 lần!
        batch_size: 256,
    };

    println!("🔥 Bắt đầu Fuzzing đi tìm Bug ẩn trong Bevy...");
    engine.run_fuzzing(50_000);
    println!("✅ Đã hoàn thành quá trình Fuzzing.");
}
