use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::{HeaderName, HeaderValue};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket as TungsteniteWebSocket, connect};

type WsStream = MaybeTlsStream<TcpStream>;

pub struct WebSocket {
  inner: TungsteniteWebSocket<WsStream>,
}

impl WebSocket {
  pub fn connect(url: &str, headers: &[(&str, String)], timeout: Duration) -> Result<Self> {
    let mut request = url
      .into_client_request()
      .with_context(|| format!("invalid websocket URL: {url}"))?;
    for (name, value) in headers {
      request.headers_mut().insert(
        HeaderName::from_bytes(name.as_bytes())?,
        HeaderValue::from_str(value)?,
      );
    }
    let (mut inner, _) = connect(request).with_context(|| format!("connect websocket {url}"))?;
    set_read_timeout(&mut inner, Some(timeout)).ok();
    set_write_timeout(&mut inner, Some(timeout)).ok();
    Ok(Self { inner })
  }

  pub fn send_text(&mut self, text: &str) -> Result<()> {
    self.inner.send(Message::Text(text.to_string().into()))?;
    Ok(())
  }

  pub fn recv_text(&mut self, timeout: Duration) -> Result<Option<String>> {
    set_read_timeout(&mut self.inner, Some(timeout)).ok();
    loop {
      match self.inner.read() {
        Ok(Message::Text(text)) => return Ok(Some(text.to_string())),
        Ok(Message::Binary(bytes)) => {
          return Ok(Some(String::from_utf8_lossy(&bytes).into_owned()));
        }
        Ok(Message::Close(_)) => return Ok(None),
        Ok(Message::Ping(payload)) => {
          let _ = self.inner.send(Message::Pong(payload));
        }
        Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
        Err(tungstenite::Error::Io(err))
          if matches!(
            err.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
          ) =>
        {
          return Ok(None);
        }
        Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
          return Ok(None);
        }
        Err(err) => return Err(err.into()),
      }
    }
  }

  pub fn close(&mut self) {
    let _ = self.inner.close(None);
  }
}

fn set_read_timeout(
  ws: &mut TungsteniteWebSocket<WsStream>,
  timeout: Option<Duration>,
) -> io::Result<()> {
  match ws.get_mut() {
    MaybeTlsStream::Plain(stream) => stream.set_read_timeout(timeout),
    MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(timeout),
    _ => Ok(()),
  }
}

fn set_write_timeout(
  ws: &mut TungsteniteWebSocket<WsStream>,
  timeout: Option<Duration>,
) -> io::Result<()> {
  match ws.get_mut() {
    MaybeTlsStream::Plain(stream) => stream.set_write_timeout(timeout),
    MaybeTlsStream::Rustls(stream) => stream.sock.set_write_timeout(timeout),
    _ => Ok(()),
  }
}

pub fn fetch_json(url: &str, timeout: Duration) -> Result<Value> {
  let client = reqwest::blocking::Client::builder()
    .no_proxy()
    .timeout(timeout)
    .build()?;
  let response = client
    .get(url)
    .header(reqwest::header::ACCEPT, "application/json")
    .send()
    .with_context(|| format!("GET {url}"))?;
  response
    .json()
    .with_context(|| format!("parse JSON from {url}"))
}

pub fn fetch_targets(cdp_list_url: &str) -> Result<Vec<Value>> {
  let value = fetch_json(cdp_list_url, Duration::from_secs(5))?;
  value
    .as_array()
    .cloned()
    .ok_or_else(|| anyhow!("unexpected Chrome DevTools target list: {value:?}"))
}

pub fn cdp_base_url(cdp_list_url: &str) -> Result<String> {
  let parsed = url::Url::parse(cdp_list_url)?;
  let host = parsed
    .host_str()
    .ok_or_else(|| anyhow!("CDP URL has no host"))?;
  let mut base = format!("{}://{}", parsed.scheme(), host);
  if let Some(port) = parsed.port() {
    base.push_str(&format!(":{port}"));
  }
  Ok(base)
}

pub fn open_new_tab(cdp_list_url: &str, url: &str) -> Result<Value> {
  let endpoint = format!(
    "{}/json/new?{}",
    cdp_base_url(cdp_list_url)?,
    urlencoding::encode(url)
  );
  let client = reqwest::blocking::Client::builder().no_proxy().build()?;
  let response = client
    .put(&endpoint)
    .send()
    .with_context(|| format!("PUT {endpoint}"))?;
  Ok(response.json()?)
}

pub struct CdpClient {
  ws: WebSocket,
  next_id: u64,
  pub contexts: HashMap<i64, Value>,
  debug: bool,
}

impl CdpClient {
  pub fn connect(websocket_url: &str, debug: bool) -> Result<Self> {
    Ok(Self {
      ws: WebSocket::connect(websocket_url, &[], Duration::from_secs(15))?,
      next_id: 1,
      contexts: HashMap::new(),
      debug,
    })
  }

  pub fn close(&mut self) {
    self.ws.close();
  }

  pub fn call(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
    let id = self.next_id;
    self.next_id += 1;
    self.ws.send_text(&serde_json::to_string(
      &json!({"id": id, "method": method, "params": params}),
    )?)?;
    let deadline = Instant::now() + timeout;
    loop {
      let remaining = deadline.saturating_duration_since(Instant::now());
      if remaining.is_zero() {
        bail!("CDP call timed out: {method}");
      }
      let message = self.recv_message(remaining)?;
      if message.get("id").and_then(Value::as_u64) != Some(id) {
        self.handle_event(&message);
        continue;
      }
      if let Some(error) = message.get("error") {
        bail!("CDP {method} failed: {error}");
      }
      return Ok(message.get("result").cloned().unwrap_or_else(|| json!({})));
    }
  }

  pub fn evaluate(&mut self, expression: &str, timeout_ms: u64) -> Result<Value> {
    let result = self.call(
      "Runtime.evaluate",
      json!({
          "expression": expression,
          "returnByValue": true,
          "timeout": timeout_ms,
      }),
      Duration::from_millis(timeout_ms + 5000),
    )?;
    if let Some(details) = result.get("exceptionDetails") {
      bail!("remote JavaScript failed: {details}");
    }
    let remote = result.get("result").cloned().unwrap_or(Value::Null);
    if let Some(value) = remote.get("value") {
      Ok(value.clone())
    } else if remote.get("type").and_then(Value::as_str) == Some("undefined") {
      Ok(Value::Null)
    } else {
      Ok(remote)
    }
  }

  pub fn pump_for(&mut self, duration: Duration) -> Result<()> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
      let _ = self.pump(Duration::from_millis(100))?;
    }
    Ok(())
  }

  pub fn pump(&mut self, timeout: Duration) -> Result<bool> {
    match self.recv_message(timeout) {
      Ok(message) => {
        self.handle_event(&message);
        Ok(true)
      }
      Err(err) if is_timeout_error(&err) => Ok(false),
      Err(err) => Err(err),
    }
  }

  fn recv_message(&mut self, timeout: Duration) -> Result<Value> {
    let Some(raw) = self.ws.recv_text(timeout)? else {
      bail!("CDP websocket closed or timed out");
    };
    if self.debug {
      eprintln!("[cdp] {}", raw.chars().take(500).collect::<String>());
    }
    Ok(serde_json::from_str(&raw)?)
  }

  fn handle_event(&mut self, message: &Value) {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    if method == "Runtime.executionContextCreated"
      && let Some(context) = message.pointer("/params/context")
      && context.pointer("/auxData/type").and_then(Value::as_str) == Some("default")
      && let Some(id) = context.get("id").and_then(Value::as_i64)
    {
      self.contexts.insert(id, context.clone());
    }
  }
}

fn is_timeout_error(err: &anyhow::Error) -> bool {
  let message = err.to_string();
  message.contains("timed out") || message.contains("timeout")
}
