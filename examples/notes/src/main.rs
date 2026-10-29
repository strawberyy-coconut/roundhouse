use notes_example::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        // Logs go to stderr so `serve --openapi` can write clean JSON to stdout.
        .with_writer(std::io::stderr)
        .init();

    notes_example::app(AppState::default())
        .serve("0.0.0.0:3000")
        .await?;
    Ok(())
}
