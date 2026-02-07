use http::HeaderMap;

/// Filters out sensitive HTTP headers that should be managed by the local server framework (Axum/Hyper).
/// 
/// When proxying requests, certain headers from the backend AI provider can conflict with 
/// the local server's behavior or cause client-side errors:
/// - `Content-Length` & `Transfer-Encoding`: Managed by Axum based on the response body we provide. 
///   Duplicating or mismatching these will cause protocol errors.
/// - `Connection` & `Keep-Alive`: Managed by the server's HTTP implementation. Forwarding 'close' 
///   from a backend will prematurely terminate the client's connection.
/// - `Content-Encoding`: Since we often decode the backend response (e.g. for logging or adaptation), 
///   forwarding the original encoding header while sending uncompressed data will break the client.
pub fn filter_proxy_headers(headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::new();
    for (name, value) in headers.iter() {
        let name_str = name.as_str().to_lowercase();
        if name_str == "content-length"
            || name_str == "transfer-encoding"
            || name_str == "connection"
            || name_str == "content-encoding"
            || name_str == "keep-alive"
            || name_str == "host"
        {
            continue;
        }
        filtered.insert(name.clone(), value.clone());
    }
    filtered
}
