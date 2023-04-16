mod converter;
mod gilrs_system;
mod rumble;

use bevy_app::{App, CoreSet, Plugin, StartupSet};
use bevy_ecs::prelude::*;
use bevy_input::InputSystem;
use bevy_utils::tracing::error;
use gilrs::GilrsBuilder;
use gilrs_system::{gilrs_event_startup_system, gilrs_event_system};
use rumble::{play_gilrs_rumble, RumblesManager};

#[derive(Default)]
pub struct GilrsPlugin;

impl Plugin for GilrsPlugin {
    fn build(&self, app: &mut App) {
        match GilrsBuilder::new()
            .with_default_filters(false)
            .set_update_state(false)
            .build()
        {
            Ok(gilrs) => {
                app.insert_non_send_resource(gilrs)
                    .init_non_send_resource::<RumblesManager>()
                    .add_startup_system(
                        gilrs_event_startup_system.in_base_set(StartupSet::PreStartup),
                    )
                    .add_system(
                        gilrs_event_system
                            .before(InputSystem)
                            .in_base_set(CoreSet::PreUpdate),
                    )
                    .add_system(play_gilrs_rumble.in_base_set(CoreSet::PostUpdate));
            }
            Err(err) => error!("Failed to start Gilrs. {}", err),
        }
    }
}
