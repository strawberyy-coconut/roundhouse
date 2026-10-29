use validation_example::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    validation_example::app(AppState)
        .serve("0.0.0.0:3003")
        .await?;
    Ok(())
}
