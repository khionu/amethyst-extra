extern crate amethyst;
#[macro_use]
extern crate serde;
extern crate ron;
#[macro_use]
extern crate log;
extern crate crossterm;
extern crate dirty;
extern crate fern;
extern crate partial_function;
extern crate rand;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate derive_new;
#[macro_use]
extern crate specs_derive;
extern crate amethyst_rhusics;
extern crate discord_rpc_client;

use amethyst::controls::FlyControlTag;
use amethyst::controls::HideCursor;
use amethyst::controls::WindowFocus;
use amethyst::core::cgmath::EuclideanSpace;
use amethyst::core::cgmath::InnerSpace;
use amethyst::core::cgmath::Vector2;
use amethyst::core::cgmath::Vector3;
use amethyst::core::cgmath::{Basis3, Deg, Point3, Quaternion, Rotation3};
use amethyst::renderer::Camera;
use amethyst::renderer::DeviceEvent;
use amethyst::renderer::DrawFlat;
use amethyst::renderer::Event;
use amethyst::renderer::Material;
use amethyst::renderer::MaterialDefaults;
use amethyst::renderer::Mesh;
use amethyst::renderer::PngFormat;
use amethyst::renderer::PosTex;
use amethyst::renderer::ScreenDimensions;
use amethyst::renderer::Texture;
use amethyst::renderer::TextureMetadata;
use amethyst::shrev::EventChannel;
use amethyst_rhusics::collision::dbvt::query_ray;
use amethyst_rhusics::collision::Ray3;
use amethyst_rhusics::rhusics_core::physics3d::Velocity3;
use amethyst_rhusics::rhusics_core::ContactEvent;
use amethyst_rhusics::rhusics_core::ForceAccumulator;
use amethyst_rhusics::rhusics_core::NextFrame;
use amethyst_rhusics::rhusics_core::Pose;
use amethyst_rhusics::rhusics_ecs::collide3d::DynamicBoundingVolumeTree3;
use amethyst_rhusics::rhusics_ecs::physics3d::BodyPose3;
use rand::{thread_rng, Rng};

use amethyst::animation::AnimationBundle;
use amethyst::assets::*;
use amethyst::audio::{AudioBundle, SourceHandle};
use amethyst::core::cgmath::{SquareMatrix, Vector4};
use amethyst::core::timing::Time;
use amethyst::core::*;
use amethyst::ecs::*;
use amethyst::input::get_input_axis_simple;
use amethyst::input::*;
use amethyst::prelude::*;
use amethyst::ui::{UiBundle, UiText};
use amethyst::Result;
use discord_rpc_client::Client as DiscordClient;
use dirty::Dirty;
use partial_function::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::hash::Hash;
use std::io::Read as IORead;
use std::io::Write as IOWrite;
use std::iter::Cycle;
use std::marker::PhantomData;
use std::ops::{Add, Sub};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread::{sleep, spawn};
use std::time::Duration;
use std::vec::IntoIter;

use crossterm::cursor::TerminalCursor;
//use crossterm::screen::RawScreen;
use crossterm::terminal::{ClearType, Terminal};
use crossterm::{Crossterm, Screen};

lazy_static! {
    static ref CROSSTERM: Crossterm = {
        let mut screen = Screen::new(true);
        screen.disable_drop();
        Crossterm::new(&screen)
    };
}

/// Loads asset from the so-called asset packs
/// It caches assets which you can manually load or unload on demand.
///
/// Example:
/// If the folder structure looks like this
/// /assets/base/sprites/player.png
/// /assets/base/sounds/click.ogg
/// /assets/base/models/cube.obj
/// /assets/mod1/sprites/player.png
/// /assets/mod1/sounds/click.ogg
/// /assets/mod2/sounds/click.ogg
///
/// resolve_path("sprites/player.png") -> /assets/mod1/sprites/player.png
/// resolve_path("models/cube.obj") -> /assets/base/models/cube.obj
/// resolve_path("sounds/click.ogg") -> Unknown.
pub struct AssetLoader {
    base_path: String,
    default_pack: String,
    asset_packs: Vec<String>,
}

impl AssetLoader {
    pub fn new(base_path: &str, default_pack: &str) -> Self {
        let mut al = AssetLoader {
            base_path: AssetLoader::sanitize_path_trail_only(&base_path),
            default_pack: AssetLoader::sanitize_path(&default_pack),
            asset_packs: Vec::new(),
        };
        al.get_asset_packs();
        al
    }

    fn sanitize_path_trail_only(path: &str) -> String {
        let mut out = path.to_string();
        let chars = path.chars();
        let last = chars.last().unwrap();
        if last == '/' {
            let idx = out.len() - 1;
            out.remove(idx);
        }
        out
    }

    fn sanitize_path(path: &str) -> String {
        let mut out = path.to_string();
        let mut chars = path.chars();
        let first = chars.next().expect("An empty path was specified!");
        let last = chars.last().unwrap();
        if first == '/' {
            out.remove(0);
        }
        if last == '/' {
            let idx = out.len() - 1;
            out.remove(idx);
        }
        out
    }

    pub fn resolve_path(&self, path: &str) -> Option<String> {
        // Try to get from default path
        let mut res = self.resolve_path_for_pack(path, &self.default_pack);

        // Try to find overrides
        for p in &self.asset_packs {
            if p != &self.default_pack {
                if let Some(r) = self.resolve_path_for_pack(path, &p) {
                    res = Some(r);
                }
            }
        }

        res
    }

    fn resolve_path_for_pack(&self, path: &str, pack: &str) -> Option<String> {
        let mut abs = self.base_path.to_owned() + "/" + pack + "/" + &path.to_owned();
        if cfg!(windows) {
            abs = abs.replace("/", "\\");
        }

        let path = Path::new(&abs);
        if path.exists() {
            Some(abs.clone())
        } else {
            warn!("Failed to find file at path: {}", abs);
            None
        }
    }

    pub fn get_asset_packs(&mut self) -> &Vec<String> {
        let mut buf: Option<Vec<String>> = None;
        if self.asset_packs.len() == 0 {
            if let Ok(elems) = fs::read_dir(&self.base_path) {
                buf = Some(
                    elems
                        .map(|e| {
                            let path = &e.unwrap().path();
                            let tmp = &path.to_str().unwrap()[self.base_path.len()..];
                            AssetLoader::sanitize_path(&tmp)
                        }).collect(),
                );
            } else {
                error!(
                    "Failed to find base_path directory for asset loading: {}",
                    self.base_path
                );
            }
        }

        if let Some(v) = buf {
            self.asset_packs = v;
        }

        &self.asset_packs
    }

    pub fn get_asset_handle<T>(path: &str, ali: &AssetLoaderInternal<T>) -> Option<Handle<T>> {
        ali.assets.get(path).cloned()
    }

    pub fn get_asset<'a, T>(
        path: &str,
        ali: &AssetLoaderInternal<T>,
        storage: &'a AssetStorage<T>,
    ) -> Option<&'a T>
    where
        T: Asset,
    {
        if let Some(h) = AssetLoader::get_asset_handle::<T>(path, ali) {
            storage.get(&h)
        } else {
            None
        }
    }

    pub fn get_asset_or_load<'a, T, F>(
        &mut self,
        path: &str,
        format: F,
        options: F::Options,
        ali: &mut AssetLoaderInternal<T>,
        storage: &'a mut AssetStorage<T>,
        loader: &Loader,
    ) -> Option<&'a T>
    where
        T: Asset,
        F: Format<T> + 'static,
    {
        if let Some(h) = AssetLoader::get_asset_handle::<T>(path, ali) {
            return storage.get(&h);
            //return Some(a);
        }
        if let Some(h) = self.load::<T, F>(path, format, options, ali, storage, loader) {
            return storage.get(&h);
        }
        None
    }

    pub fn load<T, F>(
        &self,
        path: &str,
        format: F,
        options: F::Options,
        ali: &mut AssetLoaderInternal<T>,
        storage: &mut AssetStorage<T>,
        loader: &Loader,
    ) -> Option<Handle<T>>
    where
        T: Asset,
        F: Format<T> + 'static,
    {
        if let Some(handle) = AssetLoader::get_asset_handle(path, ali) {
            return Some(handle);
        }
        if let Some(p) = self.resolve_path(path) {
            let handle: Handle<T> = loader.load(p, format, options, (), storage);
            ali.assets.insert(String::from(path), handle.clone());
            return Some(handle);
        }
        None
    }

    /// Only removes the internal Handle<T>. To truly unload the asset, you need to drop all handles that you have to it.
    pub fn unload<T>(path: &str, ali: &mut AssetLoaderInternal<T>) {
        ali.assets.remove(path);
    }

    /*pub fn load_from_extension<T>(&mut self,path: &str,ali: &mut AssetLoaderInternal<T>, storage: &mut AssetStorage<T>, loader: Loader) -> Option<Handle<T>> where T: Asset{
        let ext = AssetLoader::extension_from_path(path);
        match ext{
            "obj" => Some(self.load::<Mesh,ObjFormat>(path,ObjFormat,ali,storage,loader)),
            _ => None,
        }
    }

    pub fn auto_load_from_extension(&mut self,path: &str,res: Resources){
        let ext = AssetLoader::extension_from_path(path);
        match ext{
            "obj" => Some(self.load_from_extension::<Mesh>(ext,res.fetch_mut::<AssetLoaderInternal<Mesh>>(),res.fetch_mut::<AssetStorage<Mesh>>(),res.fetch())),
            _ => None,
        };
    }*/

    /*pub fn extension_from_path(path: &str) -> &str{
        path.split(".").as_slice().last().clone()
    }*/
}

impl Component for AssetLoader {
    type Storage = VecStorage<Self>;
}

pub struct AssetLoaderInternal<T> {
    /// Map path to asset handle.
    pub assets: HashMap<String, Handle<T>>,
}

impl<T> Default for AssetLoaderInternal<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> AssetLoaderInternal<T> {
    pub fn new() -> Self {
        AssetLoaderInternal {
            assets: HashMap::new(),
        }
    }
}

impl<T> Component for AssetLoaderInternal<T>
where
    T: Send + Sync + 'static,
{
    type Storage = VecStorage<Self>;
}

#[cfg(test)]
mod test {
    use *;

    fn load_asset_loader() -> AssetLoader {
        AssetLoader::new(
            &format!("{}/test/assets", env!("CARGO_MANIFEST_DIR")),
            "main",
        )
    }

    #[test]
    fn path_sanitisation() {
        AssetLoader::new(
            &format!("{}/test/assets/", env!("CARGO_MANIFEST_DIR")),
            "/base/",
        );
    }

    #[test]
    fn asset_loader_resolve_unique_other() {
        let asset_loader = load_asset_loader();
        assert_eq!(asset_loader.resolve_path("config/uniqueother"),Some(format!("{}/test/assets/mod1/config/uniqueother",env!("CARGO_MANIFEST_DIR")).to_string()))
    }

    #[test]
    fn asset_loader_resolve_path_override_single() {
        let asset_loader = load_asset_loader();
        assert_eq!(asset_loader.resolve_path("config/ov1"),Some(format!("{}/test/assets/mod1/config/ov1",env!("CARGO_MANIFEST_DIR")).to_string()))
    }

    #[test]
    fn asset_loader_resolve_path_override_all() {
        let asset_loader = load_asset_loader();
        assert_eq!(asset_loader.resolve_path("config/ovall"),Some(format!("{}/test/assets/mod2/config/ovall",env!("CARGO_MANIFEST_DIR")).to_string()))
    }

    #[test]
    pub fn crossterm() {
        let terminal = CROSSTERM.terminal();
        let cursor = CROSSTERM.cursor();
        //cursor.hide();

        let mut input = CROSSTERM.input().read_async().bytes();

        let input_buf = Arc::new(Mutex::new(String::new()));
        let key_buf = [0 as u8; 32];

        start_logger(input_buf.clone());

        spawn(|| loop {
            info!("More random stuff");
            sleep(Duration::from_millis(52));
        });

        loop {
            let (_, _) = terminal.terminal_size();
            info!("random stuff");
            while let Some(Ok(b)) = input.next() {
                info!("{:?} <- Char entered!", b);
                if b == 3 {
                    // Ctrl+C = exit
                    terminal.exit();
                    return;
                } else if b == b'\n' || b == 13 {
                    //info!(">{}", input_buf.lock().unwrap());
                    let mut buffer = input_buf.lock().unwrap();
                    buffer.clear();
                    refresh_input_line(&terminal, &cursor, &buffer);
                //let input = CROSSTERM.input().read_async().bytes();
                } else if b == 127 || b == 8 {
                    // Delete || Backspace
                    let mut buffer = input_buf.lock().unwrap();
                    buffer.pop();
                    refresh_input_line(&terminal, &cursor, &buffer);
                } else {
                    let mut buffer = input_buf.lock().unwrap();
                    buffer.push(b as char);
                    refresh_input_line(&terminal, &cursor, &buffer);
                }
            }
            sleep(Duration::from_millis(100));
        }
    }

    pub fn swap_write(terminal: &Terminal, cursor: &TerminalCursor, msg: &str, input_buf: &String) {
        let (_, term_height) = terminal.terminal_size();
        cursor.goto(0, term_height);
        terminal.clear(ClearType::CurrentLine);
        terminal.write(format!("{}\r\n>{}", msg, input_buf));
        //terminal.write(format!(">{}", input_buf));
    }

    pub fn refresh_input_line(terminal: &Terminal, cursor: &TerminalCursor, input_buf: &String) {
        let (_, term_height) = terminal.terminal_size();
        cursor.goto(0, term_height);
        terminal.clear(ClearType::CurrentLine);
        terminal.write(format!(">{}", input_buf));
    }

    pub fn start_logger(input_buf: Arc<Mutex<String>>) {
        let color_config = fern::colors::ColoredLevelConfig::new();
        let terminal = CROSSTERM.terminal();
        let cursor = CROSSTERM.cursor();

        fern::Dispatch::new()
            .format(move |out, message, record| {
                out.finish(format_args!(
                    "{color}[{level}][{target}] {message}{color_reset}",
                    color = format!(
                        "\x1B[{}m",
                        color_config.get_color(&record.level()).to_fg_str()
                    ),
                    level = record.level(),
                    target = record.target(),
                    message = message,
                    color_reset = "\x1B[0m",
                ))
            }).level(log::LevelFilter::Debug)
            .chain(fern::Output::call(move |record| {
                //let color = color_config.get_color(&record.level()).to_fg_str();
                //println!("\x1B[{}m[{}][{}] {}\x1B[0m",color,record.level(),record.target(),record.args());
                //println!("{}",record.args());
                //RawScreen::into_raw_mode().unwrap();
                swap_write(
                    &terminal,
                    &cursor,
                    &format!("{}", record.args()),
                    &input_buf.lock().unwrap(),
                );
            })).apply()
            .unwrap_or_else(|_| {
                error!("Global logger already set, amethyst-extra logger not used!")
            });
    }
}

/*pub trait AssetToFormat<T> where T: Sized{
    fn get_format() -> Format<T>;
}

impl AssetToFormat<Mesh> for Mesh{
    fn get_format() -> Format<Mesh>{
        ObjFormat
    }
}*/

/// Generates a rectangle 2d mesh.
pub fn gen_rectangle_mesh(
    w: f32,
    h: f32,
    loader: &Loader,
    storage: &AssetStorage<Mesh>,
) -> Handle<Mesh> {
    let verts = gen_rectangle_vertices(w, h);
    loader.load_from_data(verts.into(), (), &storage)
}

/// Generate the vertices of a rectangle.
pub fn gen_rectangle_vertices(w: f32, h: f32) -> Vec<PosTex> {
    let data: Vec<PosTex> = vec![
        PosTex {
            position: [-w / 2., -h / 2., 0.],
            tex_coord: [0., 0.],
        },
        PosTex {
            position: [w / 2., -h / 2., 0.],
            tex_coord: [1., 0.],
        },
        PosTex {
            position: [w / 2., h / 2., 0.],
            tex_coord: [1., 1.],
        },
        PosTex {
            position: [w / 2., h / 2., 0.],
            tex_coord: [1., 1.],
        },
        PosTex {
            position: [-w / 2., h / 2., 0.],
            tex_coord: [0., 1.],
        },
        PosTex {
            position: [-w / 2., -h / 2., 0.],
            tex_coord: [0., 0.],
        },
    ];
    data
}

/// Generates vertices for a circle. The circle will be made of `resolution`
/// triangles.
pub fn generate_circle_vertices(radius: f32, resolution: usize) -> Vec<PosTex> {
    use std::f32::consts::PI;

    let mut vertices = Vec::with_capacity(resolution * 3);
    let angle_offset = 2.0 * PI / resolution as f32;
    // Helper function to generate the vertex at the specified angle.
    let generate_vertex = |angle: f32| {
        let x = angle.cos();
        let y = angle.sin();
        PosTex {
            position: [x * radius, y * radius, 0.0],
            tex_coord: [x, y],
        }
    };

    for index in 0..resolution {
        vertices.push(PosTex {
            position: [0.0, 0.0, 0.0],
            tex_coord: [0.0, 0.0],
        });

        vertices.push(generate_vertex(angle_offset * index as f32));
        vertices.push(generate_vertex(angle_offset * (index + 1) as f32));
    }

    vertices
}

pub fn material_from_color(
    color: [f32; 4],
    loader: &Loader,
    storage: &AssetStorage<Texture>,
    material_defaults: &MaterialDefaults,
) -> Material {
    let albedo = loader.load_from_data(color.into(), (), &storage);
    material_from_texture(albedo, material_defaults)
}

pub fn material_from_texture(texture: Handle<Texture>, defaults: &MaterialDefaults) -> Material {
    Material {
        albedo: texture,
        ..defaults.0.clone()
    }
}

pub fn value_near<B: Add<Output = B> + Sub<Output = B> + PartialOrd + Copy>(
    number: B,
    target: B,
    margin: B,
) -> bool {
    number >= target - margin && number <= target + margin
}

pub fn material_from_png(
    path: &str,
    loader: &Loader,
    storage: &AssetStorage<Texture>,
    material_defaults: &MaterialDefaults,
) -> Material {
    material_from_texture(
        loader.load(path, PngFormat, TextureMetadata::default(), (), &storage),
        material_defaults,
    )
}

/// Doesn't work if you run `cargo run` while you are not in the root directory
pub fn get_working_dir() -> String {
    let mut base_path = String::from(
        std::env::current_exe()
            .expect("Failed to find executable path.")
            .parent()
            .expect("Failed to get parent directory of the executable.")
            .to_str()
            .unwrap(),
    );
    if base_path.contains("target/") || base_path.contains("target\\") {
        base_path = String::from(".");
    }
    base_path
}

pub struct Music {
    pub music: Cycle<IntoIter<SourceHandle>>,
}

// TODO: Broken af dependency of TransformBundle pls fix asap lmao
pub fn amethyst_gamedata_base_2d(base: &str) -> Result<GameDataBuilder<'static, 'static>> {
    amethyst::start_logger(Default::default());

    let display_config_path = format!("{}/assets/base/config/display.ron", base);

    let key_bindings_path = format!("{}/assets/base/config/input.ron", base);

    GameDataBuilder::default()
        //.with(PrefabLoaderSystem::<MyPrefabData>::default(), "", &[])
        .with_bundle(TransformBundle::new())?
        .with_bundle(
            InputBundle::<String, String>::new().with_bindings_from_file(&key_bindings_path)?,
        )?.with_bundle(UiBundle::<String, String>::new())?
        .with_bundle(AnimationBundle::<u32, Material>::new(
            "animation_control_system",
            "sampler_interpolation_system",
        ))?.with_bundle(AudioBundle::new(|music: &mut Music| music.music.next()))?
        .with(TimedDestroySystem, "timed_destroy", &[])
        .with_basic_renderer(display_config_path, DrawFlat::<PosTex>::new(), false)
}

/*pub fn build_amethyst(game_data_builder: GameDataBuilder<'static,'static>, init_state: State<GameData<'static,'static>>) -> Result<Application<GameData<'static,'static>>>{
    let resources_directory = format!("{}/assets/base", env!("CARGO_MANIFEST_DIR"));
    let game_data = game_data_builder.build()?;
    Application::build(resources_directory, init_state)?.build(game_data)
}*/

/// If the tracked resource changes, this will be checked to make sure it is a proper time to save.
pub trait ShouldSave {
    fn save_ready(&self) -> bool;
    fn set_save_ready(&mut self, ready: bool);
}

/// System used to automatically save a Resource T to a file.
/// On load, it will attempt to load it from the file and if it fails, it will use T::default().
pub struct AutoSaveSystem<T> {
    /// Absolute path.
    save_path: String,
    _phantom_data: PhantomData<T>,
}

impl<T> AutoSaveSystem<T> {
    /// Save path is an absolute path.
    pub fn new(save_path: String) -> Self {
        AutoSaveSystem {
            save_path,
            _phantom_data: PhantomData,
        }
    }
}

impl<'a, T> System<'a> for AutoSaveSystem<T>
where
    T: Serialize + DeserializeOwned + Default + ShouldSave + Send + Sync + 'static,
{
    type SystemData = (Write<'a, Dirty<T>>,);
    fn setup(&mut self, res: &mut amethyst::ecs::Resources) {
        // attempt loading
        if let Ok(mut f) = File::open(&self.save_path) {
            let mut c = String::new();
            if let Ok(_) = f.read_to_string(&mut c) {
                if let Ok(o) = ron::de::from_str::<T>(&c) {
                    res.insert(Dirty::new(o));
                } else {
                    error!(
                        "Failed to deserialize save file: {}.\nThe file might be corrupted.",
                        self.save_path
                    );
                }
            } else {
                error!("Failed to read content of save file: {}", self.save_path);
            }
        } else {
            warn!(
                "Failed to load save file: {}. It will be created during the next save.",
                self.save_path
            );
        }
        Self::SystemData::setup(res);
    }
    fn run(&mut self, (mut d,): Self::SystemData) {
        if d.dirty() {
            d.clear();
            let v = d.read();
            let s = ron::ser::to_string(&v).expect(&format!(
                "Unable to serialize the save struct for: {}",
                self.save_path
            ));
            let mut f = File::create(&self.save_path);
            if f.is_ok() {
                let file = f.as_mut().ok().unwrap();
                let res = file.write_all(s.as_bytes());
                if res.is_err() {
                    error!(
                        "Failed to write serialized save data to the file. Error: {:?}",
                        res.err().unwrap()
                    );
                }
            } else {
                error!(
                    "Failed to create or load the save file \"{}\". Error: {:?}",
                    &self.save_path,
                    &f.err().unwrap()
                );
            }
        }
    }
}

pub struct DestroyAtTime {
    pub time: f64,
}

impl Component for DestroyAtTime {
    type Storage = VecStorage<Self>;
}

pub struct DestroyInTime {
    pub timer: f64,
}

impl Component for DestroyInTime {
    type Storage = VecStorage<Self>;
}

pub struct TimedDestroySystem;

impl<'a> System<'a> for TimedDestroySystem {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, DestroyAtTime>,
        WriteStorage<'a, DestroyInTime>,
        Read<'a, Time>,
    );
    fn run(&mut self, (entities, dat, mut dit, time): Self::SystemData) {
        for (e, d) in (&*entities, &dat).join() {
            if time.absolute_time_seconds() > d.time {
                entities.delete(e).expect("Failed to delete entity!");
            }
        }

        for (e, mut d) in (&*entities, &mut dit).join() {
            if d.timer <= 0.0 {
                entities.delete(e).expect("Failed to delete entity!");
            }
            d.timer -= time.delta_seconds() as f64;
        }
    }
}

pub struct UiTimer {
    pub start: f64,
}

impl Component for UiTimer {
    type Storage = VecStorage<Self>;
}

pub struct UiTimerSystem;

impl<'a> System<'a> for UiTimerSystem {
    type SystemData = (
        ReadStorage<'a, UiTimer>,
        WriteStorage<'a, UiText>,
        Read<'a, Time>,
    );
    fn run(&mut self, (timers, mut texts, time): Self::SystemData) {
        for (timer, mut text) in (&timers, &mut texts).join() {
            let t = time.absolute_time_seconds() - timer.start;
            text.text = t.to_string(); // Simply show seconds for now.
        }
    }
}

pub trait UiAutoText: Component {
    fn get_text(&self) -> String;
}

#[derive(Default)]
pub struct UiAutoTextSystem<T> {
    phantom: PhantomData<T>,
}

impl<'a, T> System<'a> for UiAutoTextSystem<T>
where
    T: Component + UiAutoText,
{
    type SystemData = (ReadStorage<'a, T>, WriteStorage<'a, UiText>);
    fn run(&mut self, (autotexts, mut texts): Self::SystemData) {
        for (autotext, mut text) in (&autotexts, &mut texts).join() {
            text.text = autotext.get_text();
        }
    }
}

pub struct FollowMouse;
impl Component for FollowMouse {
    type Storage = VecStorage<Self>;
}

#[derive(Default)]
pub struct FollowMouseSystem<A, B> {
    phantom: PhantomData<(A, B)>,
}

impl<'a, A, B> System<'a> for FollowMouseSystem<A, B>
where
    A: Send + Sync + Hash + Eq + 'static + Clone,
    B: Send + Sync + Hash + Eq + 'static + Clone,
{
    type SystemData = (
        ReadStorage<'a, FollowMouse>,
        WriteStorage<'a, Transform>,
        ReadStorage<'a, GlobalTransform>,
        ReadExpect<'a, ScreenDimensions>,
        ReadExpect<'a, InputHandler<A, B>>,
        ReadStorage<'a, Camera>,
    );
fn run(&mut self, (follow_mouses,mut transforms, global_transforms, dimension,input,cameras): Self::SystemData){
        fn fancy_normalize(v: f32, a: f32) -> f32 {
            // [0, a]
            // [-1,1]

            v / (0.5 * a) - 1.0
        }

        let width = dimension.width();
        let height = dimension.height();

        if let Some((x, y)) = input.mouse_position() {
            for (gt, cam) in (&global_transforms, &cameras).join() {
                // TODO: Breaks with multiple cameras :ok_hand:
                let proj = cam.proj;
                let view = gt.0;
                let pv = proj * view;
                let inv = pv.invert().expect("Failed to inverse matrix");
                let tmp: Vector4<f32> = [
                    fancy_normalize(x as f32, width),
                    -fancy_normalize(y as f32, height),
                    0.0,
                    1.0,
                ]
                    .into();
                let res = inv * tmp;

                //println!("Hopefully mouse pos in world: {:?}",res);

                for (mut transform, _) in (&mut transforms, &follow_mouses).join() {
                    transform.translation = [res.x, res.y, transform.translation.z].into();
                    //println!("set pos to {:?}",transform.translation);
                }
            }
        }
    }
}

#[derive(Deserialize)]
pub struct LootTreeNode<R> {
    pub chances: i32,
    pub result: R,
}

#[derive(Deserialize)]
pub struct LootTreeBuilder<R> {
    pub nodes: Vec<LootTreeNode<R>>,
}

impl<R: Clone + 'static> LootTreeBuilder<R> {
    pub fn new() -> Self {
        LootTreeBuilder { nodes: vec![] }
    }

    pub fn build(self) -> LootTree<R> {
        let mut f = LowerPartialFunction::new();
        let mut accum = 0;
        for n in self.nodes.into_iter() {
            let tmp = n.chances;
            f = f.with(accum, move |_| n.result.clone());
            accum = accum + tmp;
        }
        LootTree {
            partial_func: f.build(),
            max: accum,
        }
    }
}

/// A loot tree based on the lower partial function construct.
/// Each loot tree node has a chance associated with it.
///
/// Example:
/// { chance: 5, result: "item1" }
/// { chance: 2, result: "item2" }
///
/// Internally this becomes
/// [0,infinite[ -> item1
/// [5,infinite[ -> item2
/// maximum = 7 exclusive (that means 6)
///
/// Chances will effectively be:
/// [0,4] (5) -> item1
/// [5,6] (2) -> item2
pub struct LootTree<R> {
    partial_func: LowerPartialFunction<i32, R>,
    max: i32,
}

impl<R> LootTree<R> {
    pub fn roll(&self) -> Option<R> {
        let rng = thread_rng().gen_range(0, self.max);
        self.partial_func.eval(rng)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, new, Component)]
pub struct FpsMovement {
    /// The movement speed in units per second.
    pub speed: f32,
}

/// The system that manages the fly movement.
/// Generic parameters are the parameters for the InputHandler.
#[derive(new)]
pub struct FpsMovementSystemSimple<A, B> {
    /// The name of the input axis to locally move in the x coordinates.
    /// Left and right.
    right_input_axis: Option<A>,
    /// The name of the input axis to locally move in the z coordinates.
    /// Forward and backward. Please note that -z is forward when defining your input configurations.
    forward_input_axis: Option<A>,
    _phantomdata: PhantomData<B>,
}

impl<'a, A, B> System<'a> for FpsMovementSystemSimple<A, B>
where
    A: Send + Sync + Hash + Eq + Clone + 'static,
    B: Send + Sync + Hash + Eq + Clone + 'static,
{
    type SystemData = (
        Read<'a, Time>,
        WriteStorage<'a, Transform>,
        Read<'a, InputHandler<A, B>>,
        ReadStorage<'a, FpsMovement>,
        WriteStorage<'a, ForceAccumulator<Vector3<f32>, Vector3<f32>>>,
    );

    fn run(&mut self, (time, transforms, input, tags, mut forces): Self::SystemData) {
        let x = get_input_axis_simple(&self.right_input_axis, &input);
        let z = get_input_axis_simple(&self.forward_input_axis, &input);

        let dir = Vector3::new(x, 0.0, z);
        if dir.magnitude() != 0.0 {
            for (transform, tag, mut force) in (&transforms, &tags, &mut forces).join() {
                let mut dir = transform.rotation * dir;
                dir = dir.normalize();
                force.add_force(dir * tag.speed * time.delta_seconds());
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Component)]
pub struct RotationControl {
    pub mouse_accum_x: f32,
    pub mouse_accum_y: f32,
}

/// The system that manages the view rotation.
/// Controlled by the mouse.
#[derive(Debug, new)]
pub struct FPSRotationRhusicsSystem<A, B> {
    sensitivity_x: f32,
    sensitivity_y: f32,
    _marker1: PhantomData<A>,
    _marker2: PhantomData<B>,
    #[new(default)]
    event_reader: Option<ReaderId<Event>>,
}

impl<'a, A, B> System<'a> for FPSRotationRhusicsSystem<A, B>
where
    A: Send + Sync + Hash + Eq + Clone + 'static,
    B: Send + Sync + Hash + Eq + Clone + 'static,
{
    type SystemData = (
        Read<'a, EventChannel<Event>>,
        WriteStorage<'a, Transform>,
        WriteStorage<'a, BodyPose3<f32>>,
        WriteStorage<'a, NextFrame<BodyPose3<f32>>>,
        WriteStorage<'a, RotationControl>,
        Read<'a, WindowFocus>,
        Read<'a, HideCursor>,
        ReadStorage<'a, FlyControlTag>,
    );

    fn run(
        &mut self,
        (
            events,
            mut transforms,
            mut body_poses,
            mut next_body_poses,
            mut rotation_controls,
            focus,
            hide,
            fly_controls,
        ): Self::SystemData,
    ) {
        let focused = focus.is_focused;
        for event in events.read(&mut self.event_reader.as_mut().unwrap()) {
            if focused && hide.hide {
                if let Event::DeviceEvent { ref event, .. } = *event {
                    if let DeviceEvent::MouseMotion { delta: (x, y) } = *event {
                        for (mut transform, mut rotation_control) in
                            (&mut transforms, &mut rotation_controls).join()
                        {
                            rotation_control.mouse_accum_x -= x as f32 * self.sensitivity_x;
                            rotation_control.mouse_accum_y += y as f32 * self.sensitivity_y;
                            // Limit maximum vertical angle to prevent locking the quaternion and/or going upside down.
                            rotation_control.mouse_accum_y =
                                rotation_control.mouse_accum_y.max(-89.5).min(89.5);

                            transform.rotation =
                                Quaternion::from_angle_x(Deg(-rotation_control.mouse_accum_y));

                            for (mut body_pose, mut next_body_pose, _) in
                                (&mut body_poses, &mut next_body_poses, &fly_controls).join()
                            {
                                body_pose.set_rotation(Quaternion::from_angle_y(Deg(
                                    rotation_control.mouse_accum_x,
                                )));
                                next_body_pose
                                    .value
                                    .set_rotation(Quaternion::from_angle_y(Deg(
                                        rotation_control.mouse_accum_x
                                    )));
                            }
                        }
                    }
                }
            }
        }
    }

    fn setup(&mut self, res: &mut Resources) {
        Self::SystemData::setup(res);
        self.event_reader = Some(res.fetch_mut::<EventChannel<Event>>().register_reader());
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, new, Component)]
pub struct Grounded {
    #[new(value = "false")]
    pub ground: bool,
    #[new(default)]
    pub since: f64,
    pub distance_check: f32,
    /// Checks if the selected entity collides with the ground.
    #[serde(skip)]
    pub watch_entity: Option<Entity>,
}

/// T: ObjectType for collider checks
#[derive(new)]
pub struct GroundCheckerSystem<T> {
    pub collider_types: Vec<T>,
    #[new(default)]
    contact_reader: Option<ReaderId<ContactEvent<Entity, Point3<f32>>>>,
}

impl<'a, T: Component + PartialEq> System<'a> for GroundCheckerSystem<T> {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Transform>,
        WriteStorage<'a, Grounded>,
        ReadStorage<'a, T>,
        Read<'a, DynamicBoundingVolumeTree3<f32>>,
        Read<'a, Time>,
        Read<'a, EventChannel<ContactEvent<Entity, Point3<f32>>>>,
    );

    fn setup(&mut self, mut res: &mut Resources) {
        Self::SystemData::setup(&mut res);
        /*self.contact_reader = Some(
            res.fetch_mut::<EventChannel<ContactEvent<Entity, Point3<f32>>>>()
                .register_reader(),
        );*/
    }

    fn run(
        &mut self,
        (entities, transforms, mut grounded, objecttypes, tree, time, contacts): Self::SystemData,
    ) {
        let down = -Vector3::unit_y();
        for (entity, transform, mut grounded) in (&*entities, &transforms, &mut grounded).join() {
            let mut ground = false;

            let ray = Ray3::new(Point3::from_vec(transform.translation), down);

            // For all in ray
            for (v, p) in query_ray(&*tree, ray) {
                // Not self and close enough
                if v.value != entity
                    && (transform.translation - Vector3::new(p.x, p.y, p.z)).magnitude()
                        <= grounded.distance_check
                {
                    // If we can jump off that type of collider
                    if let Some(obj_type) = objecttypes.get(v.value) {
                        if self.collider_types.contains(obj_type) {
                            ground = true;
                        }
                    }
                    //info!("hit bounding volume of {:?} at point {:?}", v.value, p);
                }
            }

            /*info!("run {:?}", entity);
            // Check for secondary collider if any
            for contact in contacts.read(&mut self.contact_reader.as_mut().unwrap()) {
                info!("Contact {:?} -> {:?}",contact.bodies.0, contact.bodies.1);
                // Here because we need to empty the contacts EventChannel
                if let Some(secondary) = grounded.watch_entity {
                    info!("Secondary");
                    if contact.bodies.0 == entity || contact.bodies.1 == entity {
                        // We hit our player... let's ignore that.
                        continue;
                    }
                    info!("tmp1");
                    if contact.bodies.0 != secondary && contact.bodies.1 != secondary {
                        // This has nothing to do with the secondary collider. Skip!
                        continue;
                    }
                    info!("type check");
                    let type1 = objecttypes.get(contact.bodies.0);
                    let type2 = objecttypes.get(contact.bodies.1);

                    if type1.is_none() || type2.is_none() {
                        continue;
                    }
                    info!("good to go");
                    // If we can jump off that type of collider
                    if self.collider_types.contains(type1.unwrap())
                        || self.collider_types.contains(type2.unwrap())
                    {
                        ground = true;
                    }
                }
            }*/

            if ground && !grounded.ground {
                // Just grounded
                grounded.since = time.absolute_time_seconds();
            }
            grounded.ground = ground;
        }
    }
}

#[derive(Default, Component, new)]
pub struct Jump {
    pub absolute: bool,
    pub check_ground: bool,
    pub jump_force: f32,
    pub auto_jump: bool,
    #[new(value = "0.3333")]
    pub jump_cooldown: f64,
    #[new(value = "0.1")]
    pub input_cooldown: f64,
    /// Multiplier. Time can go in the negatives.
    #[new(default)]
    pub jump_timing_boost: Option<PartialFunction<f64, f32>>,
    #[new(default)]
    pub last_jump: f64,
    #[new(default)]
    pub last_jump_offset: f64,
}

#[derive(Default)]
pub struct JumpSystem {
    /// The last time the system considered a valid jump input.
    last_logical_press: f64,
    /// Was the jump key pressed last frame?
    input_hold: bool,
    /// The last time we physically pressed the jump key.
    last_physical_press: f64,
}

impl<'a> System<'a> for JumpSystem {
    type SystemData = (
        Entities<'a>,
        WriteStorage<'a, Grounded>,
        WriteStorage<'a, Jump>,
        Read<'a, Time>,
        Read<'a, InputHandler<String, String>>,
        WriteStorage<'a, ForceAccumulator<Vector3<f32>, Vector3<f32>>>,
        WriteStorage<'a, NextFrame<Velocity3<f32>>>,
    );

    fn run(
        &mut self,
        (entities, mut grounded, mut jumps, time, input, mut forces, mut velocities): Self::SystemData,
){
        if let Some(true) = input.action_is_down("jump") {
            if !self.input_hold {
                // We just started pressing the key. Registering time.
                self.last_physical_press = time.absolute_time_seconds();
                self.input_hold = true;
            }

            for (entity, mut jump, mut force, mut velocity) in
                (&*entities, &mut jumps, &mut forces, &mut velocities).join()
            {
                // Holding the jump key on a non-auto jump controller.
                if self.input_hold && !jump.auto_jump {
                    continue;
                }

                // The last time we jumped wasn't long enough ago
                if time.absolute_time_seconds() - self.last_logical_press < jump.input_cooldown {
                    continue;
                }
                self.last_logical_press = time.absolute_time_seconds();

                // If we need to check for it, verify that we are on the ground.
                let mut grounded_since = time.absolute_time_seconds();
                if jump.check_ground {
                    if let Some(ground) = grounded.get(entity) {
                        if !ground.ground {
                            continue;
                        }
                        grounded_since = ground.since;
                    } else {
                        continue;
                    }
                }

                if time.absolute_time_seconds() - jump.last_jump > jump.jump_cooldown {
                    // Jump!
                    jump.last_jump = time.absolute_time_seconds();
                    // Offset for jump. Positive = time when we jumped AFTER we hit the ground.
                    jump.last_jump_offset = grounded_since - self.last_physical_press;

                    let multiplier = if let Some(ref curve) = jump.jump_timing_boost {
                        curve.eval(jump.last_jump_offset).unwrap_or(1.0)
                    } else {
                        1.0
                    };

                    if !jump.absolute {
                        force.add_force(Vector3::<f32>::unit_y() * jump.jump_force * multiplier);
                    } else {
                        let (x, z) = {
                            let v = velocity.value.linear();
                            (v.x, v.z)
                        };
                        velocity
                            .value
                            .set_linear(Vector3::new(x, jump.jump_force, z));
                    }
                }
                if let Some(ref mut ground) = grounded.get_mut(entity) {
                    ground.ground = false;
                }
            }
        } else {
            // The jump key was released.
            self.input_hold = false;
        }
    }
}

/// The settings controlling how the entity controlled by the `BhopMovementSystem` will behave.
/// This is a component that you should add on the entity.
#[derive(Serialize, Deserialize, Debug, Clone, Component, new)]
pub struct BhopMovement3D {
    /// False = Forces, True = Velocity
    pub absolute: bool,
    /// Use world coordinates XYZ.
    #[new(default)]
    pub absolute_axis: bool,
    /// Negates the velocity when pressing the key opposite to the current velocity.
    /// Effectively a way to instantly stop, even at high velocities.
    #[new(default)]
    pub counter_impulse: bool,
    /// Acceleration in unit/s² while on the ground.
    pub accelerate_ground: f32,
    /// Acceleration in unit/s² while in the air.
    pub accelerate_air: f32,
    /// The maximum ground velocity.
    pub max_velocity_ground: f32,
    /// The maximum air velocity.
    pub max_velocity_air: f32,
    /// Enables accelerating over maximumVelocity by airstrafing. Bunnyhop in a nutshell.
    pub allow_projection_acceleration: bool,
}

/// The system that manages the first person movements (with added projection acceleration capabilities).
/// Generic parameters are the parameters for the InputHandler.
#[derive(new)]
pub struct BhopMovementSystem<A, B> {
    /// The name of the input axis to locally move in the x coordinates.
    right_input_axis: Option<A>,
    /// The name of the input axis to locally move in the z coordinates.
    forward_input_axis: Option<A>,
    phantom_data: PhantomData<B>,
}

impl<'a, A, B> System<'a> for BhopMovementSystem<A, B>
where
    A: Send + Sync + Hash + Eq + Clone + 'static,
    B: Send + Sync + Hash + Eq + Clone + 'static,
{
    type SystemData = (
        Read<'a, Time>,
        Read<'a, InputHandler<A, B>>,
        ReadStorage<'a, Transform>,
        ReadStorage<'a, BhopMovement3D>,
        ReadStorage<'a, Grounded>,
        WriteStorage<'a, NextFrame<Velocity3<f32>>>,
    );

    fn run(
        &mut self,
        (time, input, transforms, movements, groundeds, mut velocities): Self::SystemData,
    ) {
        let x = get_input_axis_simple(&self.right_input_axis, &input);
        let z = get_input_axis_simple(&self.forward_input_axis, &input);
        let input = Vector2::new(x, z);

        if input.magnitude() != 0.0 {
            for (transform, movement, grounded, mut velocity) in
                (&transforms, &movements, &groundeds, &mut velocities).join()
            {
                let (acceleration, max_velocity) = if grounded.ground {
                    (movement.accelerate_ground, movement.max_velocity_ground)
                } else {
                    (movement.accelerate_air, movement.max_velocity_air)
                };

                // Global to local coords.
                let mut relative = SquareMatrix::invert(Basis3::from(transform.rotation).as_ref())
                    .unwrap()
                    * velocity.value.linear();

                let new_vel_rel = if movement.absolute {
                    // Absolute = We immediately set the maximum velocity without checking the max speed.
                    Vector3::new(input.x * acceleration, relative.y, input.y * acceleration)
                } else {
                    let mut wish_vel = relative;

                    if movement.counter_impulse {
                        wish_vel = counter_impulse(input, wish_vel);
                    }

                    wish_vel = accelerate_vector(
                        time.delta_seconds(),
                        input,
                        wish_vel,
                        acceleration,
                        max_velocity,
                    );
                    if !movement.allow_projection_acceleration {
                        wish_vel = limit_velocity(wish_vel, max_velocity);
                    }

                    wish_vel
                };

                // Global to local coords;
                let new_vel = transform.rotation * new_vel_rel;

                // Assign the new velocity to the player
                velocity.value.set_linear(new_vel);
            }
        }
    }
}

/// The way friction is applied.
#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub enum FrictionMode {
    /// The velocity is reduced by a fixed amount each second (deceleration).
    Linear,
    /// The velocity is reduced by a fraction of the current velocity.
    /// A value of 0.2 means that approximatively 20% of the speed will be lost each second.
    /// Since it is not calculated as an integration but as discrete values, the actual slowdown will vary slightly from case to case.
    Percent,
}

/// Component you add to your entities to apply a ground friction.
/// What the friction field does is dependent on the choosen `FrictionMode`.
#[derive(Serialize, Deserialize, Clone, Debug, Component, new)]
pub struct GroundFriction3D {
    /// The amount of friction speed loss by second.
    pub friction: f32,
    /// The way friction is applied.
    pub friction_mode: FrictionMode,
    /// The time to wait after touching the ground before applying the friction.
    pub ground_time_before_apply: f64,
}

/// Applies friction (slows the velocity down) according to the `GroundFriction3D` component of your entity.
/// Your entity also needs to have a `Grounded` component (and the `GroundCheckerSystem` added to your dispatcher) to detect the ground.
/// It also needs to have a NextFrame<Velocity3<f32>> component. This is added automatically by rhusics when creating a dynamic physical entity.
pub struct GroundFrictionSystem;

impl<'a> System<'a> for GroundFrictionSystem {
    type SystemData = (
        Read<'a, Time>,
        ReadStorage<'a, Grounded>,
        ReadStorage<'a, GroundFriction3D>,
        WriteStorage<'a, NextFrame<Velocity3<f32>>>,
    );

    fn run(&mut self, (time, groundeds, frictions, mut velocities): Self::SystemData) {
        fn apply_friction_single(v: f32, friction: f32) -> f32 {
            if v.abs() <= friction {
                return 0.0;
            }
            v - friction
        }
        for (grounded, friction, mut velocity) in (&groundeds, &frictions, &mut velocities).join() {
            if grounded.ground
                && time.absolute_time_seconds() - grounded.since
                    >= friction.ground_time_before_apply
            {
                let (x, y, z) = {
                    let v = velocity.value.linear();
                    (v.x, v.y, v.z)
                };
                match friction.friction_mode {
                    FrictionMode::Linear => {
                        let slowdown = friction.friction * time.delta_seconds();
                        velocity.value.set_linear(Vector3::new(
                            apply_friction_single(x, slowdown),
                            y,
                            apply_friction_single(z, slowdown),
                        ));
                    }
                    FrictionMode::Percent => {
                        let coef = friction.friction * time.delta_seconds();
                        velocity.value.set_linear(Vector3::new(
                            apply_friction_single(x, x * coef),
                            y,
                            apply_friction_single(z, z * coef),
                        ));
                    }
                }
            }
        }
    }
}

/// Accelerates the given `relative` vector by the given `acceleration` and `input`.
/// The `maximum_velocity` is only taken into account for the projection of the acceleration vector on the `relative` vector.
/// This allows going over the speed limit by performing what is called a "strafe".
/// If your velocity is forward and have an input accelerating you to the right, the projection of
/// the acceleration vector over your current velocity will be 0. This means that the acceleration vector will be applied fully,
/// even if this makes the resulting vector's magnitude go over `max_velocity`.
pub fn accelerate_vector(
    delta_time: f32,
    input: Vector2<f32>,
    rel: Vector3<f32>,
    acceleration: f32,
    max_velocity: f32,
) -> Vector3<f32> {
    let mut o = rel;
    let input3 = Vector3::new(input.x, 0.0, input.y);
    let rel_flat = Vector3::new(rel.x, 0.0, rel.z);
    if input3.magnitude() > 0.0 {
        let proj = rel_flat.dot(input3.normalize());
        let mut accel_velocity = acceleration * delta_time as f32;
        if proj + accel_velocity > max_velocity {
            accel_velocity = max_velocity - proj;
        }
        if accel_velocity > 0.0 {
            let add_speed = input3 * accel_velocity;
            o += add_speed;
        }
    }
    o
}

/// Completely negates the velocity of a specific axis if an input is performed in the opposite direction.
pub fn counter_impulse(input: Vector2<f32>, relative_velocity: Vector3<f32>) -> Vector3<f32> {
    let mut o = relative_velocity;
    if (input.x < 0.0 && relative_velocity.x > 0.001)
        || (input.x > 0.0 && relative_velocity.x < -0.001)
    {
        o = Vector3::new(0.0, relative_velocity.y, relative_velocity.z);
    }
    if (input.y < 0.0 && relative_velocity.z < -0.001)
        || (input.y > 0.0 && relative_velocity.z > 0.001)
    {
        o = Vector3::new(relative_velocity.x, relative_velocity.y, 0.0);
    }
    o
}

/// Limits the total velocity so that its magnitude doesn't exceed `maximum_velocity`.
/// If you are using the `accelerate_vector` function, calling this will ensure that air strafing
/// doesn't allow you to go over the maximum velocity, while still keeping fluid controls.
pub fn limit_velocity(vec: Vector3<f32>, maximum_velocity: f32) -> Vector3<f32> {
    let v_flat = Vector2::new(vec.x, vec.z).magnitude();
    if v_flat > maximum_velocity && maximum_velocity != 0.0 {
        let ratio = maximum_velocity / v_flat;
        return Vector3::new(vec.x * ratio, vec.y, vec.z * ratio);
    }
    vec
}

/*pub struct NavigationButton{
    pub target: fn() -> Trans,
}

impl Component for NavigationButton{
    type Storage = VecStorage<Self>;
}*/


/// Discord Rich Presence wrapper around discord_rpc_client
/// Currently errors are not exposed by the library, so I use the log crate
/// to display errors and only return Result<T, ()> from the methods.
/// 
/// Make sure to properly create your app here: https://discordapp.com/developers/applications
/// 
/// Usage: 
/// ```rs
/// fn init_discord_rich_presence() -> Result<DiscordRichPresence,()> {
///     DiscordRichPresence::new(498979571933380609, "Main Menu", Some("large_image"), Some("Hoppin World"), None, None);
/// }
/// ```
pub struct DiscordRichPresence {
    pub rpc: Arc<Mutex<DiscordClient>>,
    state: String,
    large_image: Option<String>,
    large_image_text: Option<String>,
    small_image: Option<String>,
    small_image_text: Option<String>,
}

impl DiscordRichPresence {
    pub fn new(app_id: u64,
        state: String,
        large_image: Option<String>,
        large_image_text: Option<String>,
        small_image: Option<String>,
        small_image_text: Option<String>) -> std::result::Result<Self,()> {
        let mut rpc = DiscordClient::new(app_id);
        if let Err(e) = rpc {
            error!("Failed to create discord rich presence client: {:?}", e);
            return Err(());
        }
        rpc.as_mut().unwrap().start();
        let mut drp = DiscordRichPresence {
            rpc: Arc::new(Mutex::new(rpc.unwrap())),
            state,
            large_image,
            large_image_text,
            small_image,
            small_image_text,
        };
        drp.update();
        Ok(drp)
    }
    pub fn set_state(&mut self, state: String) {
        self.state = state;
        self.update();
    }

    pub fn update(&mut self) {
        if let Err(e) = self.rpc.lock().unwrap().set_activity(|a| 
            a.state(self.state.clone())
            .assets(|ass| {
                let mut tmp = ass;
                if let Some(ref t) = self.large_image {
                    tmp = tmp.large_image(t.clone());
                }
                if let Some(ref t) = self.large_image_text {
                    tmp = tmp.large_text(t.clone());
                }
                if let Some(ref t) = self.small_image {
                    tmp = tmp.small_image(t.clone());
                }
                if let Some(ref t) = self.small_image_text {
                    tmp = tmp.small_text(t.clone());
                }
                tmp
            })
        ){
            error!("Failed to set discord rich presence state: {}", e);
        }
    }
}

impl Drop for DiscordRichPresence {
    fn drop(&mut self) {
        if let Err(e) = self.rpc.lock().unwrap().clear_activity() {
            eprintln!("Failed to clear discord rich presence activity {:?}", e);
        }
    }
}

/// Changes the discord rich presence state, if present in the world.
pub fn set_discord_state(state: String, world: &mut World) {
    if let Some(mut drp) = world.res.try_fetch_mut::<DiscordRichPresence>() {
        drp.set_state(state);
    }
}


/*
  * = could do it in the engine directly
  BHop controller
  2D controllers
  load asset by name ("images/player.png"), infer which one to load using asset override system (modding)
  - {modname}/localisation/{player_lang}.txt
  *http calls utils
  item/inventory system

*/
