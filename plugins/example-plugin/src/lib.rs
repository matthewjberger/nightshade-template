use nightshade::plugin::{drain_engine_events, log, spawn_cube, spawn_sphere, EngineEvent};
use std::cell::RefCell;

struct GameState {
    initialized: bool,
    spawn_timer: f32,
    spawn_count: u32,
    delta_time: f32,
    frame_count: u64,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            initialized: false,
            spawn_timer: 0.0,
            spawn_count: 0,
            delta_time: 0.0,
            frame_count: 0,
        }
    }
}

thread_local! {
    static STATE: RefCell<GameState> = RefCell::new(GameState::default());
}

fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut GameState) -> R,
{
    STATE.with_borrow_mut(f)
}

#[unsafe(no_mangle)]
pub extern "C" fn on_init() {
    with_state(|state| {
        if state.initialized {
            return;
        }
        state.initialized = true;

        log("Example plugin initialized!");

        spawn_cube(0.0, 1.0, -5.0);
        spawn_sphere(2.0, 1.0, -5.0);
        spawn_cube(-2.0, 1.0, -5.0);

        log("Spawned initial objects");
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn on_frame() {
    for event in drain_engine_events() {
        match event {
            EngineEvent::FrameStart {
                delta_time,
                frame_count,
            } => {
                with_state(|state| {
                    state.delta_time = delta_time;
                    state.frame_count = frame_count;
                });
            }
            EngineEvent::KeyPressed { key_code } => {
                log(&format!("Key pressed: {}", key_code));

                if key_code == 31 {
                    spawn_sphere(0.0, 3.0, -5.0);
                    log("Spawned sphere on 'S' key");
                } else if key_code == 46 {
                    spawn_cube(0.0, 3.0, -5.0);
                    log("Spawned cube on 'C' key");
                }
            }
            EngineEvent::KeyReleased { key_code } => {
                log(&format!("Key released: {}", key_code));
            }
            _ => {}
        }
    }

    with_state(|state| {
        state.spawn_timer += state.delta_time;

        if state.spawn_timer >= 2.0 && state.spawn_count < 10 {
            state.spawn_timer = 0.0;
            state.spawn_count += 1;

            let x = (state.spawn_count as f32) * 1.5 - 7.5;
            let y = 0.5;
            let z = -8.0;

            let frame = state.frame_count;
            if state.spawn_count % 2 == 0 {
                spawn_cube(x, y, z);
                log(&format!(
                    "Frame {}: Spawned cube at ({}, {}, {})",
                    frame, x, y, z
                ));
            } else {
                spawn_sphere(x, y, z);
                log(&format!(
                    "Frame {}: Spawned sphere at ({}, {}, {})",
                    frame, x, y, z
                ));
            }
        }
    });
}
