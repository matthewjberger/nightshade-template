use crate::ecs::{TemplateResources, register_template_components};
use crate::systems::example;
use nightshade::prelude::*;

/// The game plugin. Registers the app member world, the app resources,
/// and the startup and update systems against the [`App`] builder; grow
/// your game by adding systems in `src/systems/` and registering them
/// here.
pub struct TemplatePlugin;

impl Plugin for TemplatePlugin {
    fn build(&self, app: &mut App) {
        app.world
            .expect_resource_mut::<nightshade::ecs::window::resources::Window>()
            .title = "Template".to_string();
        app.insert_resource(TemplateResources::default());
        app.add_system(Stage::Startup, initialize);
        app.add_system(Stage::Update, update);
    }
}

fn initialize(world: &mut World) {
    world.ecs.add_world_at(GAME, register_template_components());

    world
        .expect_resource_mut::<nightshade::render::config::DebugDraw>()
        .show_grid = true;
    world
        .expect_resource_mut::<nightshade::render::config::RenderSettings>()
        .atmosphere = Atmosphere::Nebula;

    spawn_sun(world);

    let camera_entity = spawn_pan_orbit_camera(
        world,
        Vec3::new(0.0, 0.0, 0.0),
        15.0,
        0.0,
        std::f32::consts::FRAC_PI_4,
        "Main Camera".to_string(),
    );
    world.resources.active_camera = Some(camera_entity);
}

fn update(template_resources: &mut TemplateResources, world: &mut World) {
    example::tick(template_resources, world);

    let events = std::mem::take(
        &mut world
            .expect_resource_mut::<nightshade::ecs::input::resources::Input>()
            .events,
    );
    for event in events {
        if let AppEvent::Keyboard { key, state } = event
            && matches!((key, state), (KeyCode::KeyQ, KeyState::Pressed))
        {
            world
                .expect_resource_mut::<nightshade::ecs::window::resources::Window>()
                .should_exit = true;
        }
    }
}
