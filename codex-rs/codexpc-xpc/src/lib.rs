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
    fn codexpc_xpc_cancel(handle: *mut c_void);
    fn codexpc_xpc_free(handle: *mut c_void);
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
