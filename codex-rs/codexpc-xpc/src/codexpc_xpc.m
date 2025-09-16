#import <Foundation/Foundation.h>
#import <xpc/xpc.h>

typedef void (*codexpc_event_cb)(void* ctx,
                                 const char* type,
                                 const char* text,
                                 const char* code,
                                 const char* message,
                                 const char* response_id,
                                 uint64_t input_tokens,
                                 uint64_t output_tokens,
                                 uint64_t total_tokens,
                                 const char* tool_name,
                                 const char* tool_input,
                                 const char* tool_output);

typedef struct codexpc_handle {
  xpc_connection_t conn;
  char req_id[64];
  codexpc_event_cb cb;
  void* ctx;
} codexpc_handle;

static void codexpc_send_create(codexpc_handle* h,
                                const char* service,
                                const char* checkpoint,
                                const char* instructions,
                                const char* conversation_json,
                                const char* tools_json,
                                const char* reasoning_json,
                                double temperature,
                                uint64_t max_tokens) {
  xpc_connection_t c = xpc_connection_create_mach_service(service, NULL, 0);
  h->conn = c;
  xpc_connection_set_event_handler(c, ^(xpc_object_t ev) {
    if (ev == NULL) return;
    if (xpc_get_type(ev) == XPC_TYPE_ERROR) {
      return;
    }
    if (xpc_get_type(ev) != XPC_TYPE_DICTIONARY) return;
    const char* rid = xpc_dictionary_get_string(ev, "req_id");
    if (!rid || strncmp(rid, h->req_id, sizeof(h->req_id)) != 0) return;
    const char* typ = xpc_dictionary_get_string(ev, "type");
    const char* text = xpc_dictionary_get_string(ev, "text");
    const char* code = xpc_dictionary_get_string(ev, "code");
    const char* message = xpc_dictionary_get_string(ev, "message");
    const char* response_id = xpc_dictionary_get_string(ev, "response_id");
    uint64_t in_toks = 0, out_toks = 0, tot_toks = 0;
    xpc_object_t usage = xpc_dictionary_get_value(ev, "token_usage");
    if (usage && xpc_get_type(usage) == XPC_TYPE_DICTIONARY) {
      in_toks = xpc_dictionary_get_uint64(usage, "input_tokens");
      out_toks = xpc_dictionary_get_uint64(usage, "output_tokens");
      tot_toks = xpc_dictionary_get_uint64(usage, "total_tokens");
    }
    // If this is an output_item.done, surface the item.type and item.status via code/message
    if (typ && strcmp(typ, "output_item.done") == 0) {
      xpc_object_t item = xpc_dictionary_get_value(ev, "item");
      if (item && xpc_get_type(item) == XPC_TYPE_DICTIONARY) {
        code = xpc_dictionary_get_string(item, "type");
        message = xpc_dictionary_get_string(item, "status");
      }
    }
    const char* tool_name = NULL;
    const char* tool_input = NULL;
    // If this is an output_item.done, surface the item.type and item.status via code/message
    if (typ && strcmp(typ, "output_item.done") == 0) {
      xpc_object_t item = xpc_dictionary_get_value(ev, "item");
      if (item && xpc_get_type(item) == XPC_TYPE_DICTIONARY) {
        code = xpc_dictionary_get_string(item, "type");
        message = xpc_dictionary_get_string(item, "status");
        tool_name = xpc_dictionary_get_string(item, "name");
        tool_input = xpc_dictionary_get_string(item, "input");
        if (!tool_input) {
          tool_input = xpc_dictionary_get_string(item, "arguments");
        }
        tool_output = xpc_dictionary_get_string(item, "output");
      }
    }
    if (h->cb) h->cb(h->ctx, typ ?: "", text, code, message, response_id, in_toks, out_toks, tot_toks, tool_name, tool_input, tool_output);
  });
  xpc_connection_resume(c);

  xpc_object_t msg = xpc_dictionary_create(NULL, NULL, 0);
  xpc_dictionary_set_string(msg, "service", service);
  xpc_dictionary_set_uint64(msg, "proto_version", 1);
  xpc_dictionary_set_string(msg, "type", "create");
  xpc_dictionary_set_string(msg, "req_id", h->req_id);
  xpc_dictionary_set_string(msg, "model", "gpt-oss");
  xpc_dictionary_set_string(msg, "checkpoint_path", checkpoint);
  if (instructions) xpc_dictionary_set_string(msg, "instructions", instructions);
  // Optional inputs/tools/reasoning as JSON (best-effort parse)
  @autoreleasepool {
    if (conversation_json) {
      xpc_dictionary_set_string(msg, "harmony_conversation", conversation_json);
    }
    if (tools_json) {
      // Presence triggers placeholder in daemon; content is ignored for now
      xpc_object_t arr = xpc_array_create(NULL, 0);
      xpc_dictionary_set_value(msg, "tools", arr);
    }
    if (reasoning_json) {
      xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
      xpc_dictionary_set_value(msg, "reasoning", d);
    }
  }
  if (max_tokens) xpc_dictionary_set_uint64(msg, "max_output_tokens", max_tokens);
  xpc_object_t sampling = xpc_dictionary_create(NULL, NULL, 0);
  xpc_dictionary_set_double(sampling, "temperature", temperature);
  xpc_dictionary_set_value(msg, "sampling", sampling);
  xpc_connection_send_message(c, msg);
}

void* codexpc_xpc_start(const char* service,
                        const char* checkpoint,
                        const char* instructions,
                        const char* conversation_json,
                        const char* tools_json,
                        const char* reasoning_json,
                        double temperature,
                        uint64_t max_tokens,
                        codexpc_event_cb cb,
                        void* ctx) {
  if (!service || !checkpoint || !cb) return NULL;
  codexpc_handle* h = calloc(1, sizeof(codexpc_handle));
  h->cb = cb;
  h->ctx = ctx;
  // Generate req_id
  @autoreleasepool {
    NSString* rid = [[NSUUID UUID] UUIDString];
    strlcpy(h->req_id, [rid UTF8String], sizeof(h->req_id));
  }
  codexpc_send_create(h, service, checkpoint, instructions, conversation_json, tools_json, reasoning_json, temperature, max_tokens);
  return h;
}

void codexpc_xpc_cancel(void* handle) {
  codexpc_handle* h = (codexpc_handle*)handle;
  if (!h || !h->conn) return;
  xpc_object_t msg = xpc_dictionary_create(NULL, NULL, 0);
  xpc_dictionary_set_string(msg, "service", "com.yourorg.codexpc");
  xpc_dictionary_set_uint64(msg, "proto_version", 1);
  xpc_dictionary_set_string(msg, "type", "cancel");
  xpc_dictionary_set_string(msg, "req_id", h->req_id);
  xpc_connection_send_message(h->conn, msg);
}

void codexpc_xpc_free(void* handle) {
  codexpc_handle* h = (codexpc_handle*)handle;
  if (!h) return;
  if (h->conn) {
    xpc_release(h->conn);
    h->conn = NULL;
  }
  free(h);
}
