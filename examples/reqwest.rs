//! Basic example of using the SSE client to connect to an endpoint
//!
//! Run with: cargo run --example reqwest --features reqwest

use futures::StreamExt;
use sseer::EventSource;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple GET request to an SSE endpoint
    let client = reqwest::Client::new();
    let request = client.get("https://example.com/events");

    // Create the EventSource
    let mut event_source = EventSource::new(request)?;

    println!("Connecting to SSE stream...");

    // Process events as they arrive
    while let Some(result) = event_source.next().await {
        match result {
            Ok(event) => {
                use sseer::reqwest::StreamEvent;
                match event {
                    StreamEvent::Open => {
                        println!("Connection opened!");
                    }
                    StreamEvent::Event(evt) => {
                        println!("Event type: {}", evt.event);
                        println!("Data: {}", evt.data);
                        if !evt.id.is_empty() {
                            println!("ID: {}", evt.id);
                        }
                        if let Some(retry) = evt.retry {
                            println!("Retry: {:?}", retry);
                        }
                        println!("---");
                    }
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);

                // Check if we should stop on certain errors
                if e.is_response_err() {
                    eprintln!("Server returned error response, stopping...");
                    break;
                }
            }
        }
    }

    println!("Stream ended");
    Ok(())
}
