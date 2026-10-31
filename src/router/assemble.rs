use std::any::TypeId;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Router;
use axum::handler::Handler;
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;

use crate::build_error::BuildError;
use crate::problem::Problem;
use crate::registry::{Module, RouteRecord};

use super::app::ValidationDefaults;
use super::built::ResolvedRoute;

pub(super) fn merge_router(
    router: Router<()>,
    merged: Router<()>,
    source: crate::build_error::Source,
) -> Result<Router<()>, BuildError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| router.merge(merged))) {
        Ok(router) => Ok(router),
        Err(payload) => Err(BuildError::RouteConflict {
            path: "<merged router>".to_string(),
            reason: format!(
                "the router given to `App::merge` collides with a declared route ({}); \
                 declare the overlapping paths through roundhouse or merge a non-overlapping router",
                panic_message(payload.as_ref())
            ),
            source,
        }),
    }
}

pub(super) fn add_get_route<H, T>(
    router: Router<()>,
    path: &'static str,
    handler: H,
    source: crate::build_error::Source,
) -> Result<Router<()>, BuildError>
where
    H: Handler<T, ()> + Clone + Send + Sync + 'static,
    T: 'static,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        router.route(path, axum::routing::get(handler))
    })) {
        Ok(router) => Ok(router),
        Err(payload) => Err(BuildError::RouteConflict {
            path: path.to_string(),
            reason: format!(
                "the generated route `{path}` collides with a route already in the assembled app \
                 ({}); move the document route or declare the conflicting path through roundhouse",
                panic_message(payload.as_ref())
            ),
            source,
        }),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "route conflict".to_string()
    }
}

pub(super) fn docs_html(spec_path: &str) -> String {
    format!(
        r##"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>API documentation</title>
  <link rel="stylesheet"
        href="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui.css"
        integrity="sha384-wxLW6kwyHktdDGr6Pv1zgm/VGJh99lfUbzSn6HNHBENZlCN7W602k9VkGdxuFvPn"
        crossorigin="anonymous">
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui-bundle.js"
          integrity="sha384-wmyclcVGX/WhUkdkATwhaK1X1JtiNrr2EoYJ+diV3vj4v6OC5yCeSu+yW13SYJep"
          crossorigin="anonymous"></script>
  <script>
    SwaggerUIBundle({{ url: "{spec_path}", dom_id: "#swagger-ui" }});
  </script>
</body>
</html>"##
    )
}

pub(super) fn build_module(
    module: &'static Module,
    routes: &HashMap<*const RouteRecord, (String, MethodRouter<()>)>,
) -> (Router<()>, bool) {
    let mut router = Router::<()>::new();
    let mut has_routes = false;

    for record in module.routes {
        if let Some((path, method_router)) = routes.get(&(record as *const RouteRecord)) {
            router = router.route(path, method_router.clone());
            has_routes = true;
        }
    }

    for child in module.children {
        let (child_router, child_has_routes) = build_module(child, routes);
        has_routes |= child_has_routes;
        router = router.merge(child_router);
    }

    if has_routes && let Some(layer) = module.layer {
        router = layer(router);
    }

    (router, has_routes)
}

pub(super) fn build_method_router<S>(
    route: &ResolvedRoute,
    state: &S,
    engine: Option<&Arc<roundhouse_schema::engine::SchemaEngine>>,
    max_body: Option<usize>,
    route_bodies: &BTreeMap<String, (TypeId, String)>,
) -> Result<MethodRouter<()>, BuildError>
where
    S: Clone + Send + Sync + 'static,
{
    let record = route.record;
    let boxed = (record.handler)();

    let mut method_router = if (record.state)().is_some() {
        let method_router =
            boxed
                .downcast::<MethodRouter<S>>()
                .map_err(|_| BuildError::HandlerBuild {
                    operation_id: record
                        .operation_id
                        .unwrap_or(record.handler_name)
                        .to_string(),
                    message: format!(
                        "registered for state `{}` but the handler was built for another state",
                        (record.state_name)()
                    ),
                    source: record.source_ref(),
                })?;
        method_router.with_state::<()>(state.clone())
    } else {
        let method_router =
            boxed
                .downcast::<MethodRouter<()>>()
                .map_err(|_| BuildError::HandlerBuild {
                    operation_id: record
                        .operation_id
                        .unwrap_or(record.handler_name)
                        .to_string(),
                    message: "registered as stateless but the handler was built for a state"
                        .to_string(),
                    source: record.source_ref(),
                })?;
        method_router.with_state::<()>(())
    };

    if !record.validate {
        method_router = method_router.layer(axum::middleware::from_fn(skip_validation));
    }
    if record.validate
        && let Some(engine) = engine
        && let Some(type_id) = route_bodies
            .get(&route.operation_id)
            .filter(|(_, content_type)| {
                content_type == "application/json" || content_type.ends_with("+json")
            })
            .map(|(type_id, _)| *type_id)
    {
        method_router = method_router.layer(axum::middleware::from_fn_with_state(
            (engine.clone(), type_id, max_body),
            validate_json_body,
        ));
    }
    if let Some(layer) = record.layer {
        method_router = layer(method_router);
    }
    // Apply guards last so they wrap route layers: a layer that short-circuits must not be able
    // to answer before the scheme is checked. A handler that already takes `Auth<A>` performs the
    // same check during extraction, and a route-entry `security = [A]` is enforced by the
    // generated shim, so skip those guards instead of authenticating twice.
    for guard in &route.auth {
        if record.required_auth.contains(&guard.name) || record.shim_auth.contains(&guard.name) {
            continue;
        }
        method_router = (guard.apply)(method_router, state);
    }
    Ok(method_router)
}

/// Validate the raw JSON body against the operation's schema before any extractor runs.
///
/// This is what makes plain `axum::Json<T>` validated without a `Serialize` bound: the engine sees
/// the same bytes the client sent, and the request is rebuilt for the extractor. Malformed JSON is
/// also turned into a `Problem` here, so the body rejection stays one shape.
async fn validate_json_body(
    axum::extract::State((engine, type_id, max_body)): axum::extract::State<(
        Arc<roundhouse_schema::engine::SchemaEngine>,
        TypeId,
        Option<usize>,
    )>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let state = crate::extract::ValidationState::read(request.extensions());
    if state.mode == crate::extract::Validation::Off || state.skip {
        return next.run(request).await;
    }

    let is_json = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let value = value.split(';').next().unwrap_or(value).trim();
            value == "application/json" || value.ends_with("+json")
        });
    if !is_json {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    let limit = state.max_body.or(max_body).unwrap_or(2 * 1024 * 1024);
    let bytes = match axum::body::to_bytes(body, limit).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Problem::from_rejection(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                "Payload Too Large",
                format!("request body exceeds {limit} bytes"),
            )
            .into_response();
        }
    };
    let value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return Problem::from_rejection(
                axum::http::StatusCode::BAD_REQUEST,
                "Bad Request",
                error.to_string(),
            )
            .into_response();
        }
    };
    if let Some(Err(errors)) = engine.validate_type(type_id, &value) {
        return Problem::from_validation_with_status(errors, state.status).into_response();
    }

    let mut parts = parts;
    let mut state = state;
    state.body_validated = true;
    parts.extensions.insert(state);
    next.run(axum::http::Request::from_parts(
        parts,
        axum::body::Body::from(bytes),
    ))
    .await
}

async fn skip_validation(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut state = crate::extract::ValidationState::read(request.extensions());
    state.skip = true;
    request.extensions_mut().insert(state);
    next.run(request).await
}

pub(super) async fn inject_validation_config(
    axum::extract::State((status, mode, max_body, max_parts, engine)): axum::extract::State<
        ValidationDefaults,
    >,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    request.extensions_mut().insert(crate::extract::ValidationState {
        status,
        mode,
        max_body,
        skip: false,
        body_validated: false,
    });
    #[cfg(feature = "multipart")]
    request
        .extensions_mut()
        .insert(crate::multipart::MultipartLimits {
            max_body,
            max_parts,
        });
    #[cfg(not(feature = "multipart"))]
    let _ = (max_body, max_parts);
    if let Some(engine) = engine {
        request.extensions_mut().insert(engine);
    }
    next.run(request).await
}
