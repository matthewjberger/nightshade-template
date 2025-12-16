use anyhow::Result;
use nightshade::ecs::prefab::{GltfLoadResult, import_gltf_from_path, spawn_prefab};
use nightshade::plugin::{EngineCommand, EngineEvent, Primitive};
use nightshade::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use wasmtime::*;

enum AsyncResult {
    File {
        plugin_id: u64,
        request_id: u64,
        result: std::io::Result<Vec<u8>>,
    },
    Texture {
        plugin_id: u64,
        request_id: u64,
        texture_name: String,
        texture_id: u64,
        result: Result<(Vec<u8>, u32, u32), String>,
    },
    Prefab {
        plugin_id: u64,
        request_id: u64,
        position: Vec3,
        result: Result<GltfLoadResult, String>,
    },
}

struct PluginInstance {
    id: u64,
    store: Store<PluginState>,
    instance: Instance,
    on_init: Option<TypedFunc<(), ()>>,
    on_frame: Option<TypedFunc<(), ()>>,
}

struct PluginState {
    wasi: wasmtime_wasi::preview1::WasiP1Ctx,
    pending_commands: Vec<EngineCommand>,
}

pub struct PluginRuntime {
    engine: Engine,
    plugins: Vec<PluginInstance>,
    plugin_id_to_index: HashMap<u64, usize>,
    entity_map: HashMap<u64, Entity>,
    reverse_entity_map: HashMap<Entity, u64>,
    texture_id_to_name: HashMap<u64, String>,
    plugins_base_path: PathBuf,
    canonical_base_path: PathBuf,
    async_sender: Sender<AsyncResult>,
    async_receiver: Receiver<AsyncResult>,
    pending_input_events: Vec<EngineEvent>,
    last_mouse_position: (f32, f32),
    next_entity_id: u64,
    next_texture_id: u64,
    next_plugin_id: u64,
    frames_since_cleanup: u64,
}

const CLEANUP_INTERVAL_FRAMES: u64 = 60;

impl PluginRuntime {
    pub fn new(plugins_base_path: PathBuf) -> Result<Self> {
        let engine = Engine::default();
        let (async_sender, async_receiver) = channel();
        let canonical_base_path =
            std::fs::canonicalize(&plugins_base_path).unwrap_or_else(|_| plugins_base_path.clone());
        Ok(Self {
            engine,
            plugins: Vec::new(),
            plugin_id_to_index: HashMap::new(),
            entity_map: HashMap::new(),
            reverse_entity_map: HashMap::new(),
            texture_id_to_name: HashMap::new(),
            plugins_base_path,
            canonical_base_path,
            async_sender,
            async_receiver,
            pending_input_events: Vec::new(),
            last_mouse_position: (0.0, 0.0),
            next_entity_id: 1,
            next_texture_id: 1,
            next_plugin_id: 1,
            frames_since_cleanup: 0,
        })
    }

    pub fn load_plugins_from_directory(&mut self, dir: &Path) -> Result<()> {
        if !dir.exists() {
            tracing::info!("Plugin directory does not exist: {:?}", dir);
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "wasm") {
                match self.load_plugin(&path) {
                    Ok(_) => tracing::info!("Loaded plugin: {:?}", path),
                    Err(error) => tracing::error!("Failed to load plugin {:?}: {}", path, error),
                }
            }
        }

        Ok(())
    }

    fn load_plugin(&mut self, path: &Path) -> Result<()> {
        let module = Module::from_file(&self.engine, path)?;
        let mut linker: Linker<PluginState> = Linker::new(&self.engine);

        wasmtime_wasi::preview1::add_to_linker_sync(&mut linker, |state| &mut state.wasi)?;

        linker.func_wrap(
            "env",
            "host_send_command",
            |mut caller: Caller<'_, PluginState>, ptr: u32, len: u32| {
                let memory = caller.get_export("memory").and_then(|e| e.into_memory());
                let Some(memory) = memory else {
                    tracing::error!("Plugin has no memory export");
                    return;
                };
                let data = memory.data(&caller);
                if (ptr as usize) + (len as usize) > data.len() {
                    tracing::error!("Plugin command buffer out of bounds");
                    return;
                }
                let bytes = data[ptr as usize..(ptr + len) as usize].to_vec();

                match EngineCommand::from_bytes(&bytes) {
                    Some(cmd) => caller.data_mut().pending_commands.push(cmd),
                    None => tracing::warn!("Failed to deserialize plugin command"),
                }
            },
        )?;

        let wasi = wasmtime_wasi::WasiCtxBuilder::new()
            .inherit_stdio()
            .build_p1();

        let state = PluginState {
            wasi,
            pending_commands: Vec::new(),
        };

        let mut store = Store::new(&self.engine, state);
        let instance = linker.instantiate(&mut store, &module)?;

        let on_init = instance
            .get_typed_func::<(), ()>(&mut store, "on_init")
            .ok();
        let on_frame = instance
            .get_typed_func::<(), ()>(&mut store, "on_frame")
            .ok();

        let plugin_id = self.generate_plugin_id();
        let index = self.plugins.len();

        self.plugins.push(PluginInstance {
            id: plugin_id,
            store,
            instance,
            on_init,
            on_frame,
        });

        self.plugin_id_to_index.insert(plugin_id, index);

        Ok(())
    }

    pub fn call_on_init(&mut self, world: &mut World) {
        let commands = self.collect_init_commands();
        self.process_commands(world, commands);
    }

    fn collect_init_commands(&mut self) -> Vec<(u64, EngineCommand)> {
        let mut commands = Vec::new();

        for plugin in &mut self.plugins {
            if let Some(ref on_init) = plugin.on_init {
                if let Err(error) = on_init.call(&mut plugin.store, ()) {
                    tracing::error!("Plugin {} on_init failed: {}", plugin.id, error);
                }
                for command in plugin.store.data_mut().pending_commands.drain(..) {
                    commands.push((plugin.id, command));
                }
            }
        }

        commands
    }

    pub fn run_frame(&mut self, world: &mut World) {
        self.process_async_results(world);
        self.cleanup_stale_entities(world);
        self.flush_pending_input_events();

        let delta_time = world.resources.window.timing.delta_time;
        let frame_count = world.resources.window.timing.frame_counter as u64;

        self.dispatch_event_to_all(&EngineEvent::FrameStart {
            delta_time,
            frame_count,
        });

        let mouse_pos = world.resources.input.mouse.position;
        let new_pos = (mouse_pos.x, mouse_pos.y);
        if new_pos != self.last_mouse_position {
            self.last_mouse_position = new_pos;
            self.dispatch_event_to_all(&EngineEvent::MouseMoved {
                x: new_pos.0,
                y: new_pos.1,
            });
        }

        let commands = self.collect_frame_commands();
        self.process_commands(world, commands);
    }

    fn collect_frame_commands(&mut self) -> Vec<(u64, EngineCommand)> {
        let mut commands = Vec::new();

        for plugin in &mut self.plugins {
            if let Some(ref on_frame) = plugin.on_frame {
                if let Err(error) = on_frame.call(&mut plugin.store, ()) {
                    tracing::error!("Plugin {} on_frame failed: {}", plugin.id, error);
                }
                for command in plugin.store.data_mut().pending_commands.drain(..) {
                    commands.push((plugin.id, command));
                }
            }
        }

        commands
    }

    pub fn queue_keyboard_event(&mut self, key_code: KeyCode, state: KeyState) {
        let event = if state.is_pressed() {
            EngineEvent::KeyPressed {
                key_code: key_code as u32,
            }
        } else {
            EngineEvent::KeyReleased {
                key_code: key_code as u32,
            }
        };
        self.pending_input_events.push(event);
    }

    pub fn queue_mouse_event(&mut self, state: ElementState, button: MouseButton) {
        let button_id = match button {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            MouseButton::Middle => 2,
            MouseButton::Back => 3,
            MouseButton::Forward => 4,
            MouseButton::Other(id) => id as u32 + 5,
        };

        let event = match state {
            ElementState::Pressed => EngineEvent::MouseButtonPressed { button: button_id },
            ElementState::Released => EngineEvent::MouseButtonReleased { button: button_id },
        };
        self.pending_input_events.push(event);
    }

    fn generate_entity_id(&mut self) -> u64 {
        let id = self.next_entity_id;
        self.next_entity_id += 1;
        id
    }

    fn generate_texture_id(&mut self) -> u64 {
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        id
    }

    fn generate_plugin_id(&mut self) -> u64 {
        let id = self.next_plugin_id;
        self.next_plugin_id += 1;
        id
    }

    fn get_plugin_by_id(&mut self, plugin_id: u64) -> Option<&mut PluginInstance> {
        self.plugin_id_to_index
            .get(&plugin_id)
            .copied()
            .and_then(|index| self.plugins.get_mut(index))
    }

    fn dispatch_event_to_all(&mut self, event: &EngineEvent) {
        let event_bytes = match event.to_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::error!("Failed to serialize event: {}", error);
                return;
            }
        };

        for plugin in &mut self.plugins {
            send_event_to_plugin(plugin, &event_bytes);
        }
    }

    fn dispatch_event_to_plugin(&mut self, plugin_id: u64, event: &EngineEvent) {
        let event_bytes = match event.to_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::error!("Failed to serialize event: {}", error);
                return;
            }
        };

        if let Some(plugin) = self.get_plugin_by_id(plugin_id) {
            send_event_to_plugin(plugin, &event_bytes);
        } else {
            tracing::warn!("Plugin {} no longer exists, dropping event", plugin_id);
        }
    }

    fn register_entity(&mut self, entity: Entity) -> u64 {
        if let Some(&existing_id) = self.reverse_entity_map.get(&entity) {
            return existing_id;
        }
        let plugin_entity_id = self.generate_entity_id();
        self.entity_map.insert(plugin_entity_id, entity);
        self.reverse_entity_map.insert(entity, plugin_entity_id);
        plugin_entity_id
    }

    fn get_entity(&self, plugin_entity_id: u64) -> Option<Entity> {
        self.entity_map.get(&plugin_entity_id).copied()
    }

    fn unregister_entity(&mut self, plugin_entity_id: u64) {
        if let Some(entity) = self.entity_map.remove(&plugin_entity_id) {
            self.reverse_entity_map.remove(&entity);
        }
    }

    fn flush_pending_input_events(&mut self) {
        let events = std::mem::take(&mut self.pending_input_events);
        for event in events {
            self.dispatch_event_to_all(&event);
        }
    }

    fn cleanup_stale_entities(&mut self, world: &World) {
        self.frames_since_cleanup += 1;
        if self.frames_since_cleanup < CLEANUP_INTERVAL_FRAMES {
            return;
        }
        self.frames_since_cleanup = 0;

        let stale_ids: Vec<u64> = self
            .entity_map
            .iter()
            .filter(|(_, entity)| !is_entity_valid(world, **entity))
            .map(|(id, _)| *id)
            .collect();

        if !stale_ids.is_empty() {
            tracing::info!("Cleaning up {} stale plugin entities", stale_ids.len());
        }

        for id in stale_ids {
            self.unregister_entity(id);
        }
    }

    fn sanitize_path(&self, path: &str) -> Option<PathBuf> {
        let path = Path::new(path);

        if path.is_absolute() {
            return None;
        }

        for component in path.components() {
            match component {
                std::path::Component::ParentDir => return None,
                std::path::Component::Prefix(_) => return None,
                std::path::Component::RootDir => return None,
                _ => {}
            }
        }

        let full_path = self.plugins_base_path.join(path);

        let canonical_full = match std::fs::canonicalize(&full_path) {
            Ok(path) => path,
            Err(_) => return None,
        };

        if !canonical_full.starts_with(&self.canonical_base_path) {
            return None;
        }

        Some(canonical_full)
    }

    fn process_async_results(&mut self, world: &mut World) {
        while let Ok(result) = self.async_receiver.try_recv() {
            match result {
                AsyncResult::File {
                    plugin_id,
                    request_id,
                    result,
                } => {
                    let event = match result {
                        Ok(data) => EngineEvent::FileLoaded { request_id, data },
                        Err(error) => EngineEvent::FileError {
                            request_id,
                            error: error.to_string(),
                        },
                    };
                    self.dispatch_event_to_plugin(plugin_id, &event);
                }
                AsyncResult::Texture {
                    plugin_id,
                    request_id,
                    texture_name,
                    texture_id,
                    result,
                } => match result {
                    Ok((rgba_data, width, height)) => {
                        world.queue_command(WorldCommand::LoadTexture {
                            name: texture_name.clone(),
                            rgba_data,
                            width,
                            height,
                        });
                        self.texture_id_to_name.insert(texture_id, texture_name);
                        let event = EngineEvent::TextureLoaded {
                            request_id,
                            texture_id,
                        };
                        self.dispatch_event_to_plugin(plugin_id, &event);
                    }
                    Err(error) => {
                        let event = EngineEvent::AssetError { request_id, error };
                        self.dispatch_event_to_plugin(plugin_id, &event);
                    }
                },
                AsyncResult::Prefab {
                    plugin_id,
                    request_id,
                    position,
                    result,
                } => match result {
                    Ok(gltf_result) => {
                        for (mesh_name, mesh) in gltf_result.meshes {
                            world.resources.mesh_cache.insert(mesh_name, mesh);
                        }
                        for (texture_name, (rgba_data, width, height)) in gltf_result.textures {
                            world.queue_command(WorldCommand::LoadTexture {
                                name: texture_name,
                                rgba_data,
                                width,
                                height,
                            });
                        }
                        if let Some(prefab) = gltf_result.prefabs.first() {
                            let entity = spawn_prefab(world, prefab, position);
                            let entity_id = self.register_entity(entity);
                            let event = EngineEvent::PrefabLoaded {
                                request_id,
                                entity_id,
                            };
                            self.dispatch_event_to_plugin(plugin_id, &event);
                        } else {
                            let event = EngineEvent::AssetError {
                                request_id,
                                error: "No prefabs found in glTF file".to_string(),
                            };
                            self.dispatch_event_to_plugin(plugin_id, &event);
                        }
                    }
                    Err(error) => {
                        let event = EngineEvent::AssetError { request_id, error };
                        self.dispatch_event_to_plugin(plugin_id, &event);
                    }
                },
            }
        }
    }

    fn process_commands(&mut self, world: &mut World, commands: Vec<(u64, EngineCommand)>) {
        for (plugin_id, command) in commands {
            match command {
                EngineCommand::Log { message } => {
                    tracing::info!("[Plugin {}] {}", plugin_id, message);
                }
                EngineCommand::SpawnPrimitive {
                    primitive,
                    x,
                    y,
                    z,
                    request_id,
                } => {
                    let pos = Vec3::new(x, y, z);
                    let entity = match primitive {
                        Primitive::Cube => spawn_cube_at(world, pos),
                        Primitive::Sphere => spawn_sphere_at(world, pos),
                        Primitive::Cylinder => spawn_cylinder_at(world, pos),
                        Primitive::Plane => spawn_plane_at(world, pos),
                        Primitive::Cone => spawn_cone_at(world, pos),
                    };
                    let entity_id = self.register_entity(entity);
                    let event = EngineEvent::EntitySpawned {
                        request_id,
                        entity_id,
                    };
                    self.dispatch_event_to_plugin(plugin_id, &event);
                }
                EngineCommand::DespawnEntity { entity_id } => match self.get_entity(entity_id) {
                    Some(entity) => {
                        world.queue_command(WorldCommand::DespawnRecursive { entity });
                        self.unregister_entity(entity_id);
                    }
                    None => {
                        tracing::warn!(
                            "Plugin {} tried to despawn invalid entity {}",
                            plugin_id,
                            entity_id
                        );
                    }
                },
                EngineCommand::SetEntityPosition { entity_id, x, y, z } => {
                    match self.get_entity(entity_id) {
                        Some(entity) => {
                            if let Some(transform) = world.mutate_local_transform(entity) {
                                transform.translation = Vec3::new(x, y, z);
                            }
                            world.set_local_transform_dirty(entity, LocalTransformDirty);
                        }
                        None => {
                            tracing::warn!(
                                "Plugin {} tried to set position on invalid entity {}",
                                plugin_id,
                                entity_id
                            );
                        }
                    }
                }
                EngineCommand::SetEntityScale { entity_id, x, y, z } => {
                    match self.get_entity(entity_id) {
                        Some(entity) => {
                            if let Some(transform) = world.mutate_local_transform(entity) {
                                transform.scale = Vec3::new(x, y, z);
                            }
                            world.set_local_transform_dirty(entity, LocalTransformDirty);
                        }
                        None => {
                            tracing::warn!(
                                "Plugin {} tried to set scale on invalid entity {}",
                                plugin_id,
                                entity_id
                            );
                        }
                    }
                }
                EngineCommand::SetEntityRotation {
                    entity_id,
                    x,
                    y,
                    z,
                    w,
                } => match self.get_entity(entity_id) {
                    Some(entity) => {
                        if let Some(transform) = world.mutate_local_transform(entity) {
                            transform.rotation = Quat::new(w, x, y, z);
                        }
                        world.set_local_transform_dirty(entity, LocalTransformDirty);
                    }
                    None => {
                        tracing::warn!(
                            "Plugin {} tried to set rotation on invalid entity {}",
                            plugin_id,
                            entity_id
                        );
                    }
                },
                EngineCommand::GetEntityPosition {
                    entity_id,
                    request_id,
                } => match self.get_entity(entity_id) {
                    Some(entity) => {
                        if let Some(transform) = world.get_local_transform(entity) {
                            let event = EngineEvent::EntityPosition {
                                request_id,
                                entity_id,
                                x: transform.translation.x,
                                y: transform.translation.y,
                                z: transform.translation.z,
                            };
                            self.dispatch_event_to_plugin(plugin_id, &event);
                        } else {
                            let event = EngineEvent::EntityNotFound {
                                request_id,
                                entity_id,
                            };
                            self.dispatch_event_to_plugin(plugin_id, &event);
                        }
                    }
                    None => {
                        let event = EngineEvent::EntityNotFound {
                            request_id,
                            entity_id,
                        };
                        self.dispatch_event_to_plugin(plugin_id, &event);
                    }
                },
                EngineCommand::GetEntityScale {
                    entity_id,
                    request_id,
                } => match self.get_entity(entity_id) {
                    Some(entity) => {
                        if let Some(transform) = world.get_local_transform(entity) {
                            let event = EngineEvent::EntityScale {
                                request_id,
                                entity_id,
                                x: transform.scale.x,
                                y: transform.scale.y,
                                z: transform.scale.z,
                            };
                            self.dispatch_event_to_plugin(plugin_id, &event);
                        } else {
                            let event = EngineEvent::EntityNotFound {
                                request_id,
                                entity_id,
                            };
                            self.dispatch_event_to_plugin(plugin_id, &event);
                        }
                    }
                    None => {
                        let event = EngineEvent::EntityNotFound {
                            request_id,
                            entity_id,
                        };
                        self.dispatch_event_to_plugin(plugin_id, &event);
                    }
                },
                EngineCommand::GetEntityRotation {
                    entity_id,
                    request_id,
                } => match self.get_entity(entity_id) {
                    Some(entity) => {
                        if let Some(transform) = world.get_local_transform(entity) {
                            let event = EngineEvent::EntityRotation {
                                request_id,
                                entity_id,
                                x: transform.rotation.i,
                                y: transform.rotation.j,
                                z: transform.rotation.k,
                                w: transform.rotation.w,
                            };
                            self.dispatch_event_to_plugin(plugin_id, &event);
                        } else {
                            let event = EngineEvent::EntityNotFound {
                                request_id,
                                entity_id,
                            };
                            self.dispatch_event_to_plugin(plugin_id, &event);
                        }
                    }
                    None => {
                        let event = EngineEvent::EntityNotFound {
                            request_id,
                            entity_id,
                        };
                        self.dispatch_event_to_plugin(plugin_id, &event);
                    }
                },
                EngineCommand::ReadFile { path, request_id } => {
                    if let Some(safe_path) = self.sanitize_path(&path) {
                        let sender = self.async_sender.clone();
                        std::thread::spawn(move || {
                            let result = std::fs::read(&safe_path);
                            let _ = sender.send(AsyncResult::File {
                                plugin_id,
                                request_id,
                                result,
                            });
                        });
                    } else {
                        tracing::warn!("Plugin {} tried to read invalid path: {}", plugin_id, path);
                        let event = EngineEvent::FileError {
                            request_id,
                            error: "Invalid path: access denied".to_string(),
                        };
                        self.dispatch_event_to_plugin(plugin_id, &event);
                    }
                }
                EngineCommand::LoadTexture { path, request_id } => {
                    if let Some(safe_path) = self.sanitize_path(&path) {
                        let sender = self.async_sender.clone();
                        let texture_name = path.clone();
                        let texture_id = self.generate_texture_id();
                        std::thread::spawn(move || {
                            let result = std::fs::read(&safe_path)
                                .map_err(|e| e.to_string())
                                .and_then(|data| {
                                    image::load_from_memory(&data)
                                        .map_err(|e| format!("Failed to decode image: {}", e))
                                        .map(|img| {
                                            let rgba = img.to_rgba8();
                                            let (width, height) = rgba.dimensions();
                                            (rgba.into_raw(), width, height)
                                        })
                                });
                            let _ = sender.send(AsyncResult::Texture {
                                plugin_id,
                                request_id,
                                texture_name,
                                texture_id,
                                result,
                            });
                        });
                    } else {
                        tracing::warn!(
                            "Plugin {} tried to load texture from invalid path: {}",
                            plugin_id,
                            path
                        );
                        let event = EngineEvent::AssetError {
                            request_id,
                            error: "Invalid path: access denied".to_string(),
                        };
                        self.dispatch_event_to_plugin(plugin_id, &event);
                    }
                }
                EngineCommand::LoadPrefab {
                    path,
                    x,
                    y,
                    z,
                    request_id,
                } => {
                    if let Some(safe_path) = self.sanitize_path(&path) {
                        let sender = self.async_sender.clone();
                        let position = Vec3::new(x, y, z);
                        std::thread::spawn(move || {
                            let result =
                                import_gltf_from_path(&safe_path).map_err(|e| e.to_string());
                            let _ = sender.send(AsyncResult::Prefab {
                                plugin_id,
                                request_id,
                                position,
                                result,
                            });
                        });
                    } else {
                        tracing::warn!(
                            "Plugin {} tried to load prefab from invalid path: {}",
                            plugin_id,
                            path
                        );
                        let event = EngineEvent::AssetError {
                            request_id,
                            error: "Invalid path: access denied".to_string(),
                        };
                        self.dispatch_event_to_plugin(plugin_id, &event);
                    }
                }
                EngineCommand::SetEntityMaterial {
                    entity_id,
                    texture_id,
                } => match self.get_entity(entity_id) {
                    Some(entity) => match self.texture_id_to_name.get(&texture_id).cloned() {
                        Some(texture_name) => {
                            if let Some(current_material) = world.get_material(entity) {
                                let mut new_material = current_material.clone();
                                new_material.base_texture = Some(texture_name.clone());
                                world.resources.texture_cache.add_reference(&texture_name);
                                set_material_with_textures(world, entity, new_material);
                            } else {
                                tracing::warn!(
                                    "Plugin {} tried to set material on entity {} without material",
                                    plugin_id,
                                    entity_id
                                );
                            }
                        }
                        None => {
                            tracing::warn!(
                                "Plugin {} tried to use invalid texture {}",
                                plugin_id,
                                texture_id
                            );
                        }
                    },
                    None => {
                        tracing::warn!(
                            "Plugin {} tried to set material on invalid entity {}",
                            plugin_id,
                            entity_id
                        );
                    }
                },
            }
        }
    }
}

fn send_event_to_plugin(plugin: &mut PluginInstance, event_bytes: &[u8]) {
    let plugin_alloc = match plugin
        .instance
        .get_typed_func::<u32, u32>(&mut plugin.store, "plugin_alloc")
    {
        Ok(func) => func,
        Err(_) => return,
    };

    let ptr = match plugin_alloc.call(&mut plugin.store, event_bytes.len() as u32) {
        Ok(ptr) => ptr,
        Err(error) => {
            tracing::error!("Plugin {} alloc failed: {}", plugin.id, error);
            return;
        }
    };

    let memory = match plugin.instance.get_memory(&mut plugin.store, "memory") {
        Some(mem) => mem,
        None => {
            tracing::error!("Plugin {} has no memory export", plugin.id);
            return;
        }
    };

    if let Err(error) = memory.write(&mut plugin.store, ptr as usize, event_bytes) {
        tracing::error!("Plugin {} memory write failed: {}", plugin.id, error);
        return;
    }

    let receive_fn = match plugin
        .instance
        .get_typed_func::<(u32, u32), ()>(&mut plugin.store, "plugin_receive_event")
    {
        Ok(func) => func,
        Err(_) => return,
    };

    if let Err(error) = receive_fn.call(&mut plugin.store, (ptr, event_bytes.len() as u32)) {
        tracing::error!("Plugin {} receive_event failed: {}", plugin.id, error);
    }
}

fn is_entity_valid(world: &World, entity: Entity) -> bool {
    world.get_local_transform(entity).is_some() || world.get_global_transform(entity).is_some()
}
