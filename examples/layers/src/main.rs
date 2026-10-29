use layers_example::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        // Logs go to stderr so `serve --openapi` can write clean JSON to stdout.
        .with_writer(std::io::stderr)
        .init();

    // Resolve the contract before constructing production state. `roundhouse openapi` and
    // `roundhouse routes` invoke this path.
    if roundhouse::document_requested() {
        return Ok(layers_example::document().emit_from_env()?);
    }

    layers_example::app(AppState).serve("0.0.0.0:3002").await?;
    Ok(())
}
