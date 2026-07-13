/// Example resource. Replace, rename, or add to this list as your game
/// grows. Resources are global per-app state that systems read and mutate.
#[derive(Default)]
pub struct ExampleState {
    pub frame_count: u64,
    pub elapsed_seconds: f32,
}
