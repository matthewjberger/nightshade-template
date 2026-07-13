use crate::ecs::{GAME, TemplateResources, register_template_components};
use crate::systems::example;
use nightshade::prelude::*;

/// The application root. Holds your app resources and forwards each
/// State trait method to system functions in `src/systems/`.
#[derive(Default)]
pub struct Template {
    pub template_resources: TemplateResources,
}

impl State for Template {
    fn initialize(&mut self, world: &mut World) {
        world.ecs.add_world_at(GAME, register_template_components());

        world.resources.window.title = "Template".to_string();
        world.resources.user_interface.enabled = true;
        world.resources.debug_draw.show_grid = true;
        world.resources.render_settings.atmosphere = Atmosphere::Nebula;

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

    fn run_systems(&mut self, world: &mut World) {
        pan_orbit_camera_system(world);
        example::tick(&mut self.template_resources, world);

        let events = std::mem::take(&mut world.resources.input.events);
        for event in events {
            if let AppEvent::Keyboard { key, state } = event
                && matches!((key, state), (KeyCode::KeyQ, KeyState::Pressed))
            {
                world.resources.window.should_exit = true;
            }
        }
    }
}
