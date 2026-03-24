mod common;
use common::test_config;
use prismproxy::server;
use tokio::net::TcpListener;

#[tokio::test]
async fn shutdown_signal_stops_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    let handle = tokio::spawn(async move {
        server::run_with_listener(listener, test_config(vec![]), async {
            rx.await.ok();
        })
        .await
        .unwrap();
    });

    // Server is running — health check works
    let resp = reqwest::get(format!("http://{addr}/health")).await.unwrap();
    assert_eq!(resp.status(), 200);

    // Send shutdown signal
    tx.send(()).unwrap();

    // Server task should complete within 1 second
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
    assert!(result.is_ok(), "server did not shut down in time");
}
