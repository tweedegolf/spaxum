use axum::{
    Router,
    extract::{Request, State},
    response::{Html, Response},
    routing::get,
};
use hyper::{StatusCode, Uri};
use hyper_util::{client::legacy::connect::HttpConnector, rt::TokioExecutor};
use memory_serve::{Asset, MemoryServe};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncBufReadExt, process::Command};
use std::{
    collections::HashMap,
    env,
    io::{BufRead},
    path::{Path, PathBuf},
    process::{Stdio, exit},
};

pub use memory_serve;

/// HTTP client to proxy request in development
type Client = hyper_util::client::legacy::Client<
    hyper_util::client::legacy::connect::HttpConnector,
    axum::body::Body,
>;

/// File names for the entrypoint files (js, css)
#[derive(Debug, Deserialize, Serialize)]
pub struct EntryFiles {
    pub js: String,
    pub css: String,
}

/// Entrypoint for the esbuild instance
type EntryPoint = String;

/// Directory to serve the assets from
type DistDir = String;

/// Engine for serving assets, either proxy to an eslint instance or serve from memory
enum SpaxumEngine {
    Proxy(EntryPoint, DistDir),
    MemoryServe(EntryFiles, MemoryServe),
}

/// Spaxum instance, holds the page title and the statis asset engine
pub struct Spaxum {
    title: String,
    engine: SpaxumEngine,
    esbuild_args: Vec<String>,
    html_template: Option<String>,
    process_index: Option<Box<dyn Fn(String) -> String>>,
}

const ESBUILD_OPTIONS: &[&str] = &[
    "--color=false",
    "--asset-names=[name]",
    "--public-path=/static/",
    "--loader:.png=file",
    "--loader:.jpg=file",
    "--loader:.jpeg=file",
    "--loader:.svg=file",
    "--loader:.gif=file",
];

/// Load the assets from the memory or proxy to an esbuild instance
/// Returns a Spaxum instance that can be used to create an axum router
#[macro_export]
macro_rules! load {
    ($title:expr) => {{
        use spaxum::memory_serve::{self, Asset};
        use std::path::Path;

        if let Some(entrypoint) = option_env!("SPAXUM_ENTRYPOINT") {
            let dist_dir = Path::new(concat!(env!("OUT_DIR"), "/dist"));

            spaxum::Spaxum::new_proxy($title, entrypoint, dist_dir)
        } else {
            let assets: &[Asset] = include!(concat!(env!("OUT_DIR"), "/spaxum.rs"));

            let entry_files = spaxum::EntryFiles {
                js: option_env!("SPAXUM_JS_ENTRY")
                    .unwrap_or_default()
                    .to_string(),
                css: option_env!("SPAXUM_CSS_ENTRY")
                    .unwrap_or_default()
                    .to_string(),
            };

            spaxum::Spaxum::new($title, assets, entry_files)
        }
    }};
}

impl Spaxum {
    /// Create a new Spaxum instance, with the page title, assets and entry files
    /// Serves the assets from memory
    pub fn new(title: &str, assets: &'static [Asset], entry_files: EntryFiles) -> Self {
        let memory_serve = MemoryServe::new(assets);

        Self {
            title: title.to_string(),
            esbuild_args: Vec::new(),
            engine: SpaxumEngine::MemoryServe(entry_files, memory_serve),
            process_index: None,
            html_template: None,
        }
    }

    /// Create a new Spaxum instance, with the page title, entrypoint and dist directory
    /// Uses esbuild to bundle the assets and serve them in development mode
    pub fn new_proxy(title: &str, entrypoint: &str, dist_dir: &Path) -> Self {
        // cleanup and ignore if directory is already empty
        let _ = std::fs::remove_dir_all(dist_dir);

        let Some(dist_dir) = dist_dir.to_str() else {
            panic!("Invalid path provided by OUT_DIR");
        };

        Self {
            title: title.to_string(),
            esbuild_args: Vec::new(),
            engine: SpaxumEngine::Proxy(entrypoint.into(), dist_dir.into()),
            process_index: None,
            html_template: None,
        }
    }

    pub fn start_proxy(&self) {
        let (entrypoint, dist_dir) = match &self.engine {
            SpaxumEngine::Proxy(entrypoint, dist_dir) => (entrypoint, dist_dir),
            _ => panic!("Invalid engine type"),
        };

        let esbuild = get_esbuild_path();

        let Ok(mut child) = Command::new(esbuild)
            .args([
                entrypoint,
                "--bundle",
                format!("--outdir={dist_dir}").as_str(),
                "--watch=forever",
                format!("--servedir={dist_dir}").as_str(),
                "--serve=127.0.0.1:8888",
                "--entry-names=index",
            ])
            .args(ESBUILD_OPTIONS)
            .args(&self.esbuild_args)
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        else {
            panic!("esbuild failed to start");
        };

        tokio::spawn( async move {
            let stdout = child.stdout.take().expect("esbuild did not have a handle to stdout");
            let mut stdout_reader = tokio::io::BufReader::new(stdout).lines();

            let stderr = child.stderr.take().expect("esbuild did not have a handle to stderr");
            let mut stderr_reader = tokio::io::BufReader::new(stderr).lines();

            loop {
                tokio::select! {
                    next_line = stdout_reader.next_line() => {
                        if let Ok(Some(line)) = next_line {
                            println!("esbuild: {line}");
                        } else {
                            eprintln!("esbuild: stdout closed");
                            break;
                        }
                    },
                    next_error_line = stderr_reader.next_line() => {
                        if let Ok(Some(line)) = next_error_line {
                            eprintln!("esbuild: {line}");
                        } else {
                            eprintln!("esbuild: stderr closed");
                            break;
                        }
                    },
                    process_result = child.wait() => {
                        match process_result {
                            Ok(exit_status) => {
                                if exit_status.success() {
                                    println!("esbuild process exited successfully");
                                } else {
                                    eprintln!("esbuild process exited with error");
                                    break;
                                }
                            }
                            Err(e) => {
                                eprintln!("esbuild process failed to exit: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        });
    }

    /// Set the HTML page title
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();

        self
    }

    /// Set the process index function, this function is called before serving the index.html
    /// This can be used to process the index.html before serving it
    pub fn set_process_html(mut self, process_index: impl Fn(String) -> String + 'static) -> Self {
        self.process_index = Some(Box::new(process_index));

        self
    }

    /// Set the HTML template, this template is used to render the index.html
    pub fn set_html_template(mut self, html_template: impl Into<String>) -> Self {
        self.html_template = Some(html_template.into());

        self
    }

    /// Set additional esbuild arguments, these arguments are passed to the esbuild instance
    pub fn set_esbuild_args(mut self, args: Vec<String>) -> Self {
        self.esbuild_args = args;

        self
    }

    /// Get the memory serve instance, this can de used to fine-tune memory serve settings
    pub fn memory_serve(&self) -> Option<&MemoryServe> {
        match &self.engine {
            SpaxumEngine::MemoryServe(_, memory_serve) => Some(memory_serve),
            _ => None,
        }
    }

    /// Get the axum router for the Spaxum instance, serves static assets (from the "/static" path)
    pub fn router<S>(self) -> Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        let html = match self.html_template {
            Some(ref html) => html,
            None => include_str!("../index.html"),
        };

        let mut html = html.replace("%TITLE%", &self.title);

        match self.engine {
            SpaxumEngine::MemoryServe(entry_files, memory_serve) => {
                html = html
                    .replace("%SCRIPT%", &entry_files.js)
                    .replace("%STYLESHEET%", &entry_files.css);

                if let Some(process_index) = self.process_index {
                    html = process_index(html);
                }

                Router::new()
                    .nest("/static", memory_serve.into_router())
                    .fallback(Html(html))
            }
            _ => {
                self.start_proxy();

                let live_reload = include_str!("../live_reload.html");

                html = html
                    .replace("%SCRIPT%", "index.js")
                    .replace("%STYLESHEET%", "index.css")
                    .replace("</body>", &format!("{live_reload}</body>"));

                if let Some(process_index) = self.process_index {
                    html = process_index(html);
                }

                let client: Client =
                    hyper_util::client::legacy::Client::<(), ()>::builder(TokioExecutor::new())
                        .build(HttpConnector::new());

                let proxy_router = Router::new()
                    .fallback(get(proxy_handler))
                    .with_state(client);

                Router::new()
                    .nest("/static", proxy_router)
                    .fallback(Html(html))
            }
        }
    }
}

/// Proxy handler for development mode, proxies requests to the esbuild dev server
async fn proxy_handler(
    State(client): State<Client>,
    mut req: Request,
) -> Result<Response, StatusCode> {
    use axum::response::IntoResponse;

    let path = req.uri().path();
    let path_query = req
        .uri()
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(path);

    let uri = format!("http://127.0.0.1:8888{}", path_query);

    let Ok(uri) = Uri::try_from(uri) else {
        return Err(StatusCode::BAD_REQUEST);
    };

    *req.uri_mut() = uri;

    Ok(client
        .request(req)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .into_response())
}

/// Esbuild manifest output structure
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Output {
    bytes: usize,
    css_bundle: Option<String>,
    entry_point: Option<String>,
}

/// Esbuild manifest structure
#[derive(Debug, Deserialize, Serialize)]
struct Manifest {
    outputs: HashMap<String, Output>,
}

impl EntryFiles {
    /// Get the entry files from the esbuild manifest file
    fn from_manifest(manifest_file: &str, entrypoint: &Path) -> Option<Self> {
        let manifest_str =
            std::fs::read_to_string(manifest_file).expect("Unable to read manifest file.");

        let manifest: Manifest =
            serde_json::from_str(&manifest_str).expect("Unmable to parse manifest file.");

        for (name, output) in manifest.outputs.iter() {
            if let Some(js) = output.entry_point.as_ref() {
                if entrypoint.to_string_lossy().ends_with(js) {
                    return Some(EntryFiles {
                        js: Path::new(name)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                        css: output
                            .css_bundle
                            .as_ref()
                            .and_then(|f| {
                                Some(Path::new(&f).file_name()?.to_string_lossy().to_string())
                            })
                            .unwrap_or_default(),
                    });
                }
            }
        }

        None
    }
}

/// Error macro for build scripts
macro_rules! error {
    ($s:expr) => {
        println!("cargo::error={}", $s);
        exit(1);
    };

    ($s:expr, $($v:tt)*) => {
        println!("cargo::error={}", format!($s, $($v)*));
        exit(1);
    };
}

/// Get the path to the esbuild executable
/// Optionally use esbuild binary shipped with spaxum, fallback the system esbuild
fn get_esbuild_path() -> PathBuf {
    if !cfg!(target_arch = "x86_64") {
        return PathBuf::from("esbuild");
    }

    let current_dir = Path::new(file!()).parent().and_then(|p| p.parent());

    match current_dir {
        Some(p) => {
            let esbuild = p.join("esbuild");

            if esbuild.exists() {
                esbuild
            } else {
                PathBuf::from("esbuild")
            }
        }
        None => PathBuf::from("esbuild"),
    }
}

/// File name to write asset metadata to
const ASSET_FILE: &str = "spaxum.rs";

/// Write the asset metadata to a file
fn write_asset_file(out_dir: &Path, code: &str) {
    let target = out_dir.join(ASSET_FILE);
    match std::fs::write(&target, code) {
        Ok(_) => {}
        Err(e) => {
            error!(
                "Unable to write asset file: {} {e:?}",
                target.to_string_lossy()
            );
        }
    }
}

/// Bundle the assets using release compilation with esbuild
/// Pass the entrypoint to the runtime for debug builds
pub fn bundle(entrypoint: &str) {
    bundle_with_args(entrypoint, &[]);
}

/// Bundle the assets using release compilation with esbuild
/// Pass the entrypoint to the runtime for debug builds
/// Optionally pass additional arguments to esbuild
pub fn bundle_with_args(entrypoint: &str, build_args: &[&str]) {
    // Log messages to cargo
    fn log(msg: &str) {
        if std::env::var("SPAXUM_QUIET") != Ok("1".to_string()) {
            println!("cargo::warning={}", msg);
        }
    }

    // Check if the entrypoint exists
    let Ok(entrypoint) = Path::new(&entrypoint).canonicalize() else {
        error!("{} not found!", entrypoint);
    };

    // Get the OUT_DIR environment variable, this is where we store compressed assets and asset metadata code
    let Some(out_dir) = env::var_os("OUT_DIR") else {
        error!("OUT_DIR not set!");
    };

    // Create neccesary paths and their string variants
    let out_dir = Path::new(&out_dir);
    let dist_dir = out_dir.join("dist");
    let dist_dir_str = dist_dir.to_string_lossy();
    let entrypoint_str = entrypoint.to_string_lossy();
    let manifest_file = out_dir.join("manifest.json");
    let manifest_file_str = manifest_file.to_string_lossy();

    // Skip bundling in debug mode, assets will be served by the esbuild dev server
    if cfg!(debug_assertions) {
        println!("cargo::rustc-env=SPAXUM_ENTRYPOINT={entrypoint_str}");
        write_asset_file(out_dir, "&[]");
        log("Skipping bundling in debug mode, assets will be served by the esbuild dev server.");
        exit(0);
    }

    // Cleanup and ignore if directory is already empty
    let _ = std::fs::remove_dir_all(&dist_dir);

    // Determine the directory of the entrypoint file, and rerun the build if it changes
    let Some(source_dir) = entrypoint.parent() else {
        error!(
            "Unable to get parent directory of entrypoint: {}",
            entrypoint_str
        );
    };

    // Rerun build script if source directory changes
    println!("cargo::rerun-if-changed={}", source_dir.to_string_lossy());

    log(&format!("Bundling {entrypoint_str} using esbuild..."));

    // Bundle assets using esbuild
    let esbuild = get_esbuild_path();
    let Ok(mut child) = std::process::Command::new(esbuild)
        .args([
            "--bundle",
            &entrypoint_str,
            &format!("--outfile={dist_dir_str}/index.js"),
            &format!("--metafile={manifest_file_str}"),
            "--entry-names=[name]-[hash]",
            "--minify",
        ])
        .args(ESBUILD_OPTIONS)
        .args(build_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        error!("esbuild failed to start");
    };

    if let Some(ref mut stdout) = child.stdout {
        for line in std::io::BufReader::new(stdout).lines() {
            let line = line.unwrap();
            log(&format!("esbuild: {line}"));
        }
    }

    if let Some(ref mut stderr) = child.stderr {
        for line in std::io::BufReader::new(stderr).lines() {
            let line = line.unwrap();
            log(&format!("esbuild error: {line}"));
        }
    }

    let Ok(status) = child.wait() else {
        error!("esbuild failed to bundle: {entrypoint_str}");
    };

    // Log errors if esbuild fails
    if !status.success() {
        error!("esbuild failed to bundle: {entrypoint_str}");
    }

    // Log success message
    log("esbuild completed successfully");

    // read contents of manifest_file as string
    let Some(entry_point) = EntryFiles::from_manifest(&manifest_file_str, &entrypoint) else {
        error!(
            "Unable to find entrypoint in manifest file: {}",
            manifest_file_str
        );
    };

    // Set environment variables for the entrypoint files
    println!("cargo::rustc-env=SPAXUM_JS_ENTRY={}", entry_point.js);
    println!("cargo::rustc-env=SPAXUM_CSS_ENTRY={}", entry_point.css);

    // Convert assets to code and write to file
    let code =
        memory_serve_core::assets_to_code(&dist_dir_str, &dist_dir, Some(out_dir), true, log);

    write_asset_file(out_dir, &code);
}
