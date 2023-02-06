//! Prints out all keyboard events.

use bevy::{input::keyboard::KeyboardInput, log::LogPlugin, prelude::*};

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        prevent_default_event_handling: false,
                        ..default()
                    }),
                    ..default()
                })
                .set(LogPlugin {
                    filter: "debug,wgpu_core=warn,wgpu_hal=warn,naga=info,winit=trace".into(),
                    level: bevy::log::Level::DEBUG,
                }),
        )
        .add_system(print_keyboard_event_system)
        .run();
}

/// This system prints out all keyboard events as they come in
fn print_keyboard_event_system(
    mut keyboard_input_events: EventReader<KeyboardInput>,
    mut received_character_events: EventReader<ReceivedCharacter>,
) {
    for event in keyboard_input_events.iter() {
        info!("{:?}", event);
    }

    for event in received_character_events.iter() {
        info!("{:?}", event);
    }
}
