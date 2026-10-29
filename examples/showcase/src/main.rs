use showcase_example::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    showcase_example::app(AppState)
        .serve("0.0.0.0:3005")
        .await?;
    Ok(())
}
