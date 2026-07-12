use axum_test::TestServer;

#[tokio::test]
async fn healthz_returns_ok() {
    let server = TestServer::new(pingward::app());
    let res = server.get("/healthz").await;
    res.assert_status_ok();
    res.assert_text("ok");
}
