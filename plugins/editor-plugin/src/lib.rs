use nightshade_editor_api::{
    drain_events, drain_inspector_requests, drain_ui_responses, log, register_inspector,
    register_menu_item, respond_inspector_ui, send_mod_info, EditorEvent, InspectorRequest,
    ModInfo, UiElement, UiResponse,
};
use std::cell::RefCell;

thread_local! {
    static STATE: RefCell<PluginState> = RefCell::new(PluginState::default());
}

struct PluginState {
    initialized: bool,
    counter: u32,
}

impl Default for PluginState {
    fn default() -> Self {
        Self {
            initialized: false,
            counter: 0,
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn on_init() {
    STATE.with_borrow_mut(|state| {
        if state.initialized {
            return;
        }
        state.initialized = true;

        send_mod_info(&ModInfo {
            id: "editor-plugin".to_string(),
            name: "Editor Plugin".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        });

        log("[Editor Plugin] Initialized");

        register_inspector("Plugin Data", "plugin_data");
        register_menu_item("Tools/My Plugin", "my_plugin_menu");
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn on_frame() {
    for event in drain_events() {
        match event {
            EditorEvent::EntitySelected { entity_id } => {
                log(&format!("[Editor Plugin] Entity {} selected", entity_id));
            }
            EditorEvent::EntityDeselected => {
                log("[Editor Plugin] Entity deselected");
            }
            EditorEvent::PanelToggled { panel } => {
                log(&format!("[Editor Plugin] Panel toggled: {:?}", panel));
            }
            EditorEvent::FrameStart { .. } => {}
        }
    }

    for request in drain_inspector_requests() {
        match request {
            InspectorRequest::RenderUi {
                inspector_id,
                entity_id,
            } => {
                if inspector_id == "plugin_data" {
                    let counter = STATE.with_borrow(|state| state.counter);
                    respond_inspector_ui(vec![
                        UiElement::Label {
                            text: "Plugin Inspector".to_string(),
                        },
                        UiElement::Separator,
                        UiElement::Label {
                            text: format!("Entity ID: {}", entity_id),
                        },
                        UiElement::Label {
                            text: format!("Counter: {}", counter),
                        },
                        UiElement::Separator,
                        UiElement::Button {
                            text: "Increment".to_string(),
                            id: "increment".to_string(),
                        },
                        UiElement::Button {
                            text: "Reset".to_string(),
                            id: "reset".to_string(),
                        },
                    ]);
                }
            }
        }
    }

    for response in drain_ui_responses() {
        match response {
            UiResponse::ButtonClicked { id } => {
                STATE.with_borrow_mut(|state| match id.as_str() {
                    "increment" => {
                        state.counter += 1;
                        log(&format!("[Editor Plugin] Counter: {}", state.counter));
                    }
                    "reset" => {
                        state.counter = 0;
                        log("[Editor Plugin] Counter reset");
                    }
                    _ => {}
                });
            }
            UiResponse::TextChanged { id, value } => {
                log(&format!("[Editor Plugin] Text changed: {} = {}", id, value));
            }
        }
    }
}
