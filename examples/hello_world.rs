use actix_axum::Server;
use axum::{routing::get, Router};
use std::net::TcpListener;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let app = Router::new().route("/", get(handler));

    Server::new(app.into_make_service())
        .listen(TcpListener::bind("127.0.0.1:3000").unwrap())
        .unwrap()
        .run()
        .await
        .unwrap();
}

async fn handler() -> &'static str {
    "Hello, world!"
}
