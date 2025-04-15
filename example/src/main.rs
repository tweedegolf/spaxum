use axum::{Router, response::Html, routing::get};

#[tokio::main]
async fn main() {
    let frontend = spaxum::load!("Example")
        .set_title("Example Site")
        .set_process_html(|html: String| html.replace("Example Site", "Example Page"));

    let app = Router::new()
        .merge(frontend.router())
        .route("/hello", get(handler));

    // run it
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();

    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn handler() -> Html<&'static str> {
    Html("<h1>Hello, World!</h1>")
}
