//! Minimal [`HttpClient`] for GPUI image loading.
//!
//! GPUI's default client rejects every request, which leaves remote favicons
//! blank. Reqwest needs a Tokio reactor, while the window runs on GPUI's own
//! executor, so each request owns a short-lived Tokio runtime on a worker
//! thread. The response body is forwarded through a channel and exposed as an
//! [`AsyncBody`] stream, keeping downloads incremental instead of buffering the
//! whole image before the first byte reaches the renderer.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::future::BoxFuture;
use futures::AsyncRead;
use gpui_kit::http_client::{AsyncBody, HttpClient, Request, Response, Url};
use http::HeaderValue;

struct BodyStream {
    receiver: tokio::sync::mpsc::Receiver<Result<Bytes, String>>,
    buffer: Bytes,
    position: usize,
}

impl AsyncRead for BodyStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        loop {
            if this.position < this.buffer.len() {
                let available = &this.buffer[this.position..];
                let count = available.len().min(output.len());
                output[..count].copy_from_slice(&available[..count]);
                this.position += count;
                return Poll::Ready(Ok(count));
            }
            match this.receiver.poll_recv(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    this.buffer = chunk;
                    this.position = 0;
                }
                Poll::Ready(Some(Err(error))) => {
                    return Poll::Ready(Err(io::Error::other(error)));
                }
                Poll::Ready(None) => return Poll::Ready(Ok(0)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

pub struct ReqwestHttpClient {
    user_agent: HeaderValue,
    proxy: Option<Url>,
}

impl ReqwestHttpClient {
    pub fn new() -> Self {
        Self::with_proxy(None)
    }

    /// Use the configured app proxy when one is available; otherwise fall back
    /// to the environment proxy the window started with.
    pub fn with_proxy(proxy: Option<Url>) -> Self {
        Self {
            user_agent: HeaderValue::from_static("chatspeed-gpui"),
            proxy: proxy.or_else(|| {
                std::env::var("HTTPS_PROXY")
                    .or_else(|_| std::env::var("https_proxy"))
                    .ok()
                    .and_then(|value| value.parse().ok())
            }),
        }
    }
}

fn fail_head(
    head: &mut Option<tokio::sync::oneshot::Sender<anyhow::Result<Response<AsyncBody>>>>,
    error: anyhow::Error,
) {
    if let Some(sender) = head.take() {
        let _ = sender.send(Err(error));
    }
}

fn forward_request(
    method: http::Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    head: tokio::sync::oneshot::Sender<anyhow::Result<Response<AsyncBody>>>,
    proxy: Option<Url>,
) -> anyhow::Result<()> {
    std::thread::Builder::new()
        .name("gpui-http-client".into())
        .spawn(move || {
            let mut head = Some(head);
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    fail_head(&mut head, error.into());
                    return;
                }
            };
            runtime.block_on(async move {
                // The client owns hyper's connector, which captures the current
                // Tokio reactor. Build it inside this runtime: constructing it on
                // the GPUI thread made the first web request panic with
                // "there is no reactor running".
                let mut builder =
                    reqwest::Client::builder().redirect(reqwest::redirect::Policy::limited(10));
                if let Some(proxy) = proxy.as_ref() {
                    let proxy = match reqwest::Proxy::all(proxy.as_str()) {
                        Ok(proxy) => proxy,
                        Err(error) => {
                            fail_head(&mut head, error.into());
                            return;
                        }
                    };
                    builder = builder.proxy(proxy);
                }
                let client = match builder.build() {
                    Ok(client) => client,
                    Err(error) => {
                        fail_head(&mut head, error.into());
                        return;
                    }
                };
                let method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
                    Ok(method) => method,
                    Err(error) => {
                        fail_head(&mut head, error.into());
                        return;
                    }
                };
                let url = match reqwest::Url::parse(&url) {
                    Ok(url) => url,
                    Err(error) => {
                        fail_head(&mut head, error.into());
                        return;
                    }
                };
                let mut request = client.request(method, url);
                for (name, value) in headers {
                    request = request.header(name, value);
                }
                let mut response = match request.body(body).send().await {
                    Ok(response) => response,
                    Err(error) => {
                        fail_head(&mut head, error.into());
                        return;
                    }
                };
                let status = match http::StatusCode::from_u16(response.status().as_u16()) {
                    Ok(status) => status,
                    Err(error) => {
                        fail_head(&mut head, error.into());
                        return;
                    }
                };
                let mut builder = http::Response::builder().status(status);
                if let Some(target) = builder.headers_mut() {
                    for (name, value) in response.headers() {
                        let Ok(name) = http::HeaderName::from_bytes(name.as_str().as_bytes())
                        else {
                            continue;
                        };
                        let Ok(value) = http::HeaderValue::from_bytes(value.as_bytes()) else {
                            continue;
                        };
                        target.append(name, value);
                    }
                }
                let (sender, receiver) = tokio::sync::mpsc::channel(16);
                let stream = BodyStream {
                    receiver,
                    buffer: Bytes::new(),
                    position: 0,
                };
                let response_head = match builder.body(AsyncBody::from_reader(stream)) {
                    Ok(response) => response,
                    Err(error) => {
                        fail_head(&mut head, error.into());
                        return;
                    }
                };
                if let Some(sender_head) = head.take() {
                    if sender_head.send(Ok(response_head)).is_err() {
                        return;
                    }
                }
                loop {
                    match response.chunk().await {
                        Ok(Some(chunk)) => {
                            if sender.send(Ok(chunk)).await.is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            let _ = sender.send(Err(error.to_string())).await;
                            break;
                        }
                    }
                }
            });
        })?;
    Ok(())
}

impl HttpClient for ReqwestHttpClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        self.proxy.as_ref()
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let proxy = self.proxy.clone();
        Box::pin(async move {
            use futures::AsyncReadExt;
            let (parts, mut body) = request.into_parts();
            let mut payload = Vec::new();
            body.read_to_end(&mut payload).await?;
            let headers = parts
                .headers
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.as_str().to_owned(), value.to_owned()))
                })
                .collect();
            let (sender, receiver) = tokio::sync::oneshot::channel();
            forward_request(
                parts.method,
                parts.uri.to_string(),
                headers,
                payload,
                sender,
                proxy,
            )?;
            receiver.await?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ReqwestHttpClient;
    use gpui_kit::http_client::HttpClient;

    #[test]
    fn client_exposes_a_user_agent() {
        let client = ReqwestHttpClient::new();
        assert_eq!(
            client.user_agent().and_then(|value| value.to_str().ok()),
            Some("chatspeed-gpui")
        );
    }
}
