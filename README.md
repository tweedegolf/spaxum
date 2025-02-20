# Spaxum

Bundle and serve your SPA (Spingle Page Application) using [Axum](https://github.com/tokio-rs/axum), a Rust a web application framework that focuses on ergonomics and modularity.

Spaxum uses [esbuild](https://esbuild.github.io/) to bundle and serve frontend assets during develoopment or to bundle and minify in production.
In release builds [memory serve](https://github.com/tweedegolf/memory-serve) embeds all assets in the binary and serves them from memory at runtime.

## Usage

Create a `build.rs` file in your project and add a call to spaxum with the path to your javascript entry file:

`build.rs`

```rust
fn main() {
    spaxum::bundle("./frontend/src/app.tsx");
}
```

Load spaxum in your Axum application and merge the resulting router:

`main.rs`

```rust
use axum::{response::Html, routing::get, Router};

#[tokio::main]
async fn main() {
    let frontend = spaxum::load!("Example Site");

    let app = Router::new()
        .merge(frontend.router())
        .route("/hello", get(handler));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}

async fn handler() -> Html<&'static str> {
    Html("<h1>Hello, World!</h1>")
}
```

Note that spaxum will will a `index.html` file that loads the bundled javascript file(s) and css stylescheets.

## Caveats

Spaxum:

- (an memory serve) are opinionated, they serve a specific use-case: bundle and serve simple SPA's with your Rust backend
- strives for zero, or minimal, configuration
- strives to minimize the number of dependencies needed to bundle a javascript frontend
- uses `esbuild` and and relies on the features provided by `esbuild`
- does not work well if there are many or large frontend assets (since they are all loaded in memory at runtime)
- automatically compresses assets, both in the binary and at runtime
- spaxum ships with a precompiled version of esbuild (x86 64) and relies on a esbuild binary in your PATH as a fallback
