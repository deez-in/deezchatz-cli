use deezchatz_sdk_rust::oauth as pkce;
use std::time::Duration;
use tiny_http::{Response, Server};
use url::Url;

pub struct OAuthResult {
    pub code: String,
    pub verifier: String,
    pub redirect_uri: String,
}

pub fn login_via_browser(
    google_client_id: &str,
    port: u16,
) -> Result<OAuthResult, String> {
    let redirect_uri = format!("http://127.0.0.1:{}/callback", port);
    let verifier = pkce::generate_code_verifier();
    let challenge = pkce::generate_code_challenge(&verifier);

    let auth_url = pkce::build_google_auth_url(
        google_client_id,
        &redirect_uri,
        &challenge,
        &["openid", "email", "profile"],
    );

    let server = Server::http(format!("127.0.0.1:{}", port))
        .map_err(|e| format!("Failed to start local listener on port {}: {}", port, e))?;

    tracing::info!("Opening browser to: {}", auth_url);
    if let Err(e) = open::that(&auth_url) {
        tracing::warn!("Failed to open browser automatically: {}", e);
    }

    // Wait for the redirect callback
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(180); // 3 minutes timeout

    while start_time.elapsed() < timeout {
        match server.recv_timeout(Duration::from_millis(500)) {
            Ok(Some(request)) => {
                let req_url = request.url().to_string();
                if let Ok(parsed_url) = Url::parse(&format!("http://127.0.0.1:{}", req_url.trim_start_matches('/')))
                    .or_else(|_| Url::parse(&format!("http://127.0.0.1{}", req_url)))
                {
                    if parsed_url.path() == "/callback" {
                        let mut code = None;
                        for (k, v) in parsed_url.query_pairs() {
                            if k == "code" {
                                code = Some(v.into_owned());
                                break;
                            }
                        }

                        if let Some(auth_code) = code {
                            let html = r#"<!DOCTYPE html>
<html>
<head><title>DeezChatz Login</title></head>
<body style="font-family: sans-serif; text-align: center; padding-top: 50px; background-color: #121212; color: #fff;">
  <h1 style="color: #4CAF50;">Authentication Successful!</h1>
  <p>You can now close this tab and return to the terminal.</p>
</body>
</html>"#;
                            let response = Response::from_string(html)
                                .with_header(tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]).unwrap());
                            let _ = request.respond(response);

                            return Ok(OAuthResult {
                                code: auth_code,
                                verifier,
                                redirect_uri,
                            });
                        } else {
                            let err_html = "<h1>Authentication Failed</h1><p>Missing code parameter.</p>";
                            let _ = request.respond(Response::from_string(err_html));
                        }
                    } else {
                        let _ = request.respond(Response::from_string("Not Found").with_status_code(404));
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!("Server error: {}", e);
            }
        }
    }

    Err("OAuth login timed out after 3 minutes".to_string())
}
