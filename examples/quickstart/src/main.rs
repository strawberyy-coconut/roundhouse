//! Serve the quickstart application.

use quickstart_example::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    quickstart_example::app(AppState::default())
        .serve("0.0.0.0:3000")
        .await?;
    Ok(())
}
