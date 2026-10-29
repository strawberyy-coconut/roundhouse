use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

async fn tag(request: Request, next: Next, name: &'static str, value: &'static str) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(name, HeaderValue::from_static(value));
    response
}

pub async fn global(request: Request, next: Next) -> Response {
    tag(request, next, "x-global", "global").await
}

pub async fn module(request: Request, next: Next) -> Response {
    tag(request, next, "x-module", "module").await
}

pub async fn route_layer(request: Request, next: Next) -> Response {
    tag(request, next, "x-route-layer", "route-layer").await
}

pub async fn entry(request: Request, next: Next) -> Response {
    tag(request, next, "x-entry", "entry").await
}
