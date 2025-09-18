#![cfg(target_os = "macos")]

use std::ffi::{c_char, c_double, c_void, CStr, CString};
use tokio::sync::mpsc;

#[allow(improper_ctypes)]
extern "C" {
    fn codexpc_xpc_start(
        service: *const c_char,
        checkpoint: *const c_char,
        instructions: *const c_char,
        conversation_json: *const c_char,
        tools_json: *const c_char,
        reasoning_json: *const c_char,
        temperature: c_double,
        max_tokens: u64,
        cb: extern "C" fn(
            ctx: *mut c_void,
            typ: *const c_char,
            text: *const c_char,
            code: *const c_char,
            message: *const c_char,
            response_id: *const c_char,
            input_tokens: u64,
            output_tokens: u64,
            total_tokens: u64,
            tool_name: *const c_char,
            tool_input: *const c_char,
            tool_output: *const c_char,
        ),
        ctx: *mut c_void,
    ) -> *mut c_void;
    fn codexpc_xpc_start_from_messages(
        service: *const c_char,
        checkpoint: *const c_char,
        messages_json: *const c_char,
        tools_json: *const c_char,
        reasoning_json: *const c_char,
        temperature: c_double,
        max_tokens: u64,
        cb: extern "C" fn(
            ctx: *mut c_void,
            typ: *const c_char,
            text: *const c_char,
            code: *const c_char,
            message: *const c_char,
            response_id: *const c_char,
            input_tokens: u64,
            output_tokens: u64,
            total_tokens: u64,
            tool_name: *const c_char,
            tool_input: *const c_char,
            tool_output: *const c_char,
        ),
        ctx: *mut c_void,
    ) -> *mut c_void;
    fn codexpc_xpc_cancel(handle: *mut c_void);
    fn codexpc_xpc_free(handle: *mut c_void);
    fn codexpc_xpc_start_from_tokens(
        service: *const c_char,
        checkpoint: *const c_char,
        tokens: *const u32,
        tokens_len: usize,
        prime_final: bool,
        tools_json: *const c_char,
        reasoning_json: *const c_char,
        temperature: c_double,
        max_tokens: u64,
        cb: extern "C" fn(
            ctx: *mut c_void,
            typ: *const c_char,
            text: *const c_char,
            code: *const c_char,
            message: *const c_char,
            response_id: *const c_char,
            input_tokens: u64,
            output_tokens: u64,
            total_tokens: u64,
            tool_name: *const c_char,
            tool_input: *const c_char,
            tool_output: *const c_char,
        ),
        ctx: *mut c_void,
    ) -> *mut c_void;
    fn codexpc_xpc_handshake(service: *const c_char) -> *mut c_char;
}

#[derive(Debug, Clone)]
pub enum Event {
    Created,
    OutputTextDelta(String),
    Completed {
        response_id: String,
        input_tokens: u64,
        output_tokens: u64,
        total_tokens: u64,
    },
    Metrics {
        ttfb_ms: u64,
        tokens_per_sec: f64,
        delta_count: u64,
        tool_calls: u64,
    },
    Error {
        code: String,
        message: String,
    },
    OutputItemDone {
        item_type: String,
        status: String,
        name: String,
        input: String,
        call_id: Option<String>,
    },
    OutputItemOutput {
        name: String,
        output: String,
        call_id: Option<String>,
    },
}

pub struct Handle {
    raw: *mut c_void,
}

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { codexpc_xpc_free(self.raw) }
    }
}

extern "C" fn on_event(
    ctx: *mut c_void,
    typ: *const c_char,
    text: *const c_char,
    code: *const c_char,
    message: *const c_char,
    response_id: *const c_char,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    tool_name: *const c_char,
    tool_input: *const c_char,
    tool_output: *const c_char,
) {
    let tx = unsafe { &*(ctx as *const mpsc::UnboundedSender<Event>) };
    let t = unsafe { CStr::from_ptr(typ) }.to_string_lossy().to_string();
    match t.as_str() {
        "created" => {
            let _ = tx.send(Event::Created);
        }
        "output_text.delta" => {
            if !text.is_null() {
                let s = unsafe { CStr::from_ptr(text) }
                    .to_string_lossy()
                    .to_string();
                let _ = tx.send(Event::OutputTextDelta(s));
            }
        }
        "completed" => {
            let id = if response_id.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(response_id) }
                    .to_string_lossy()
                    .to_string()
            };
            let _ = tx.send(Event::Completed {
                response_id: id,
                input_tokens,
                output_tokens,
                total_tokens,
            });
        }
        "output_item.done" => {
            let item_type = if code.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(code) }
                    .to_string_lossy()
                    .to_string()
            };
            let status = if message.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(message) }
                    .to_string_lossy()
                    .to_string()
            };
            let name = if tool_name.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(tool_name) }
                    .to_string_lossy()
                    .to_string()
            };
            let input = if tool_input.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(tool_input) }
                    .to_string_lossy()
                    .to_string()
            };
            let call_id = if response_id.is_null() {
                None
            } else {
                let s = unsafe { CStr::from_ptr(response_id) }
                    .to_string_lossy()
                    .to_string();
                if s.is_empty() { None } else { Some(s) }
            };
            if item_type == "tool_call.output" {
                let output = if tool_output.is_null() {
                    String::new()
                } else {
                    unsafe { CStr::from_ptr(tool_output) }
                        .to_string_lossy()
                        .to_string()
                };
                let _ = tx.send(Event::OutputItemOutput { name, output, call_id });
            } else {
                let _ = tx.send(Event::OutputItemDone {
                    item_type,
                    status,
                    name,
                    input,
                    call_id,
                });
            }
        }
        "metrics" => {
            if !text.is_null() {
                let s = unsafe { CStr::from_ptr(text) }.to_string_lossy().to_string();
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                    let ttfb_ms = v.get("ttfb_ms").and_then(|x| x.as_u64()).unwrap_or(0);
                    let tokens_per_sec = v.get("tokens_per_sec").and_then(|x| x.as_f64()).unwrap_or(0.0);
                    let delta_count = v.get("delta_count").and_then(|x| x.as_u64()).unwrap_or(0);
                    let tool_calls = v.get("tool_calls").and_then(|x| x.as_u64()).unwrap_or(0);
                    let _ = tx.send(Event::Metrics { ttfb_ms, tokens_per_sec, delta_count, tool_calls });
                }
            }
        }
        "error" => {
            let c = if code.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(code) }
                    .to_string_lossy()
                    .to_string()
            };
            let m = if message.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(message) }
                    .to_string_lossy()
                    .to_string()
            };
            let _ = tx.send(Event::Error {
                code: c,
                message: m,
            });
        }
        _ => {}
    }
}

#[allow(dead_code)]
pub fn stream(
    service: &str,
    checkpoint: &str,
    instructions: &str,
    conversation_json: Option<&str>,
    tools_json: Option<&str>,
    reasoning_json: Option<&str>,
    temperature: f64,
    max_tokens: u64,
) -> (Handle, mpsc::UnboundedReceiver<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let tx_box: Box<mpsc::UnboundedSender<Event>> = Box::new(tx);
    let ctx = Box::into_raw(tx_box) as *mut c_void;

    let s = CString::new(service).unwrap();
    let ck = CString::new(checkpoint).unwrap();
    let ins = CString::new(instructions).unwrap_or_else(|_| CString::new("").unwrap());
    let raw = unsafe {
        codexpc_xpc_start(
            s.as_ptr(),
            ck.as_ptr(),
            ins.as_ptr(),
            conversation_json
                .map(|s| CString::new(s).unwrap())
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(std::ptr::null()),
            tools_json
                .map(|s| CString::new(s).unwrap())
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(std::ptr::null()),
            reasoning_json
                .map(|s| CString::new(s).unwrap())
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(std::ptr::null()),
            temperature as c_double,
            max_tokens,
            on_event,
            ctx,
        )
    };
    let handle = Handle { raw };
    (handle, rx)
}

pub fn cancel(handle: &Handle) {
    unsafe { codexpc_xpc_cancel(handle.raw) }
}

#[allow(dead_code)]
pub fn stream_from_messages(
    service: &str,
    checkpoint: &str,
    messages_json: Option<&str>,
    tools_json: Option<&str>,
    reasoning_json: Option<&str>,
    temperature: f64,
    max_tokens: u64,
) -> (Handle, mpsc::UnboundedReceiver<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let tx_box: Box<mpsc::UnboundedSender<Event>> = Box::new(tx);
    let ctx = Box::into_raw(tx_box) as *mut c_void;

    let s = CString::new(service).unwrap();
    let ck = CString::new(checkpoint).unwrap();
    let raw = unsafe {
        codexpc_xpc_start_from_messages(
            s.as_ptr(),
            ck.as_ptr(),
            messages_json
                .map(|s| CString::new(s).unwrap())
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(std::ptr::null()),
            tools_json
                .map(|s| CString::new(s).unwrap())
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(std::ptr::null()),
            reasoning_json
                .map(|s| CString::new(s).unwrap())
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(std::ptr::null()),
            temperature as c_double,
            max_tokens,
            on_event,
            ctx,
        )
    };
    let handle = Handle { raw };
    (handle, rx)
}

pub fn stream_from_tokens(
    service: &str,
    checkpoint: &str,
    tokens: &[u32],
    prime_final: bool,
    tools_json: Option<&str>,
    reasoning_json: Option<&str>,
    temperature: f64,
    max_tokens: u64,
) -> (Handle, mpsc::UnboundedReceiver<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let tx_box: Box<mpsc::UnboundedSender<Event>> = Box::new(tx);
    let ctx = Box::into_raw(tx_box) as *mut c_void;

    let s = CString::new(service).unwrap();
    let ck = CString::new(checkpoint).unwrap();
    let raw = unsafe {
        codexpc_xpc_start_from_tokens(
            s.as_ptr(),
            ck.as_ptr(),
            tokens.as_ptr(),
            tokens.len(),
            prime_final,
            tools_json
                .map(|s| CString::new(s).unwrap())
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(std::ptr::null()),
            reasoning_json
                .map(|s| CString::new(s).unwrap())
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(std::ptr::null()),
            temperature as c_double,
            max_tokens,
            on_event,
            ctx,
        )
    };
    let handle = Handle { raw };
    (handle, rx)
}

#[derive(Debug, Clone)]
pub struct Handshake {
    pub encoding_name: Option<String>,
    pub special_tokens: Vec<String>,
    pub stop_tokens_for_assistant_actions: Vec<u32>,
}

pub fn handshake(service: &str) -> Option<Handshake> {
    let s = CString::new(service).ok()?;
    let ptr = unsafe { codexpc_xpc_handshake(s.as_ptr()) };
    if ptr.is_null() {
        return None;
    }
    let json = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_string_lossy().to_string();
    unsafe { libc::free(ptr as *mut libc::c_void) };
    let val: serde_json::Value = serde_json::from_str(&json).ok()?;
    let encoding_name = val
        .get("encoding_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let special_tokens = val
        .get("special_tokens")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|e| e.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let stop_tokens_for_assistant_actions = val
        .get("stop_tokens_for_assistant_actions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|e| e.as_u64().map(|u| u as u32)).collect())
        .unwrap_or_default();
    Some(Handshake { encoding_name, special_tokens, stop_tokens_for_assistant_actions })
}
