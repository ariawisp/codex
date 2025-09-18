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
        // Reuse response_id slot to carry call_id for tool events
        const char* call_id = xpc_dictionary_get_string(item, "call_id");
        if (call_id) { response_id = call_id; }
      }
    }
    const char* tool_name = NULL;
    const char* tool_input = NULL;
    const char* tool_output = NULL;
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

static void codexpc_send_create_from_messages(codexpc_handle* h,
                                              const char* service,
                                              const char* checkpoint,
                                              const char* messages_json,
                                              const char* tools_json,
                                              const char* reasoning_json,
                                              double temperature,
                                              uint64_t max_tokens) {
  xpc_connection_t c = xpc_connection_create_mach_service(service, NULL, 0);
  h->conn = c;
  xpc_connection_set_event_handler(c, ^(xpc_object_t ev) {
    if (ev == NULL) return;
    if (xpc_get_type(ev) == XPC_TYPE_ERROR) { return; }
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
    const char* tool_name = NULL;
    const char* tool_input = NULL;
    const char* tool_output = NULL;
    if (typ && strcmp(typ, "output_item.done") == 0) {
      xpc_object_t item = xpc_dictionary_get_value(ev, "item");
      if (item && xpc_get_type(item) == XPC_TYPE_DICTIONARY) {
        code = xpc_dictionary_get_string(item, "type");
        message = xpc_dictionary_get_string(item, "status");
        tool_name = xpc_dictionary_get_string(item, "name");
        tool_input = xpc_dictionary_get_string(item, "input");
        if (!tool_input) { tool_input = xpc_dictionary_get_string(item, "arguments"); }
        tool_output = xpc_dictionary_get_string(item, "output");
        const char* call_id = xpc_dictionary_get_string(item, "call_id");
        if (call_id) { response_id = call_id; }
      }
    }
    if (h->cb) h->cb(h->ctx, typ ?: "", text, code, message, response_id, in_toks, out_toks, tot_toks, tool_name, tool_input, tool_output);
  });
  xpc_connection_resume(c);

  xpc_object_t msg = xpc_dictionary_create(NULL, NULL, 0);
  xpc_dictionary_set_string(msg, "service", service);
  xpc_dictionary_set_uint64(msg, "proto_version", 1);
  xpc_dictionary_set_string(msg, "type", "create_from_messages");
  xpc_dictionary_set_string(msg, "req_id", h->req_id);
  xpc_dictionary_set_string(msg, "model", "gpt-oss");
  xpc_dictionary_set_string(msg, "checkpoint_path", checkpoint);

  @autoreleasepool {
    if (messages_json) {
      NSData* data = [NSData dataWithBytes:messages_json length:strlen(messages_json)];
      NSError* err = nil;
      id root = [NSJSONSerialization JSONObjectWithData:data options:0 error:&err];
      if (err == nil && [root isKindOfClass:[NSDictionary class]]) {
        NSArray* arr = ((NSDictionary*)root)[@"messages"];
        if ([arr isKindOfClass:[NSArray class]]) {
          xpc_object_t xarr = xpc_array_create(NULL, 0);
          for (id m in (NSArray*)arr) {
            if (![m isKindOfClass:[NSDictionary class]]) continue;
            NSDictionary* md = (NSDictionary*)m;
            const char* role = NULL;
            id r = md[@"role"]; if ([r isKindOfClass:[NSString class]]) role = [(NSString*)r UTF8String];
            if (!role) continue;
            xpc_object_t xm = xpc_dictionary_create(NULL, NULL, 0);
            xpc_dictionary_set_string(xm, "role", role);
            id name = md[@"name"]; if ([name isKindOfClass:[NSString class]]) xpc_dictionary_set_string(xm, "name", [(NSString*)name UTF8String]);
            id rec = md[@"recipient"]; if ([rec isKindOfClass:[NSString class]]) xpc_dictionary_set_string(xm, "recipient", [(NSString*)rec UTF8String]);
            id ch = md[@"channel"]; if ([ch isKindOfClass:[NSString class]]) xpc_dictionary_set_string(xm, "channel", [(NSString*)ch UTF8String]);
            id ct = md[@"content_type"]; if ([ct isKindOfClass:[NSString class]]) xpc_dictionary_set_string(xm, "content_type", [(NSString*)ct UTF8String]);
            id content = md[@"content"];
            if ([content isKindOfClass:[NSArray class]]) {
              xpc_object_t xc = xpc_array_create(NULL, 0);
              for (id p in (NSArray*)content) {
                if (![p isKindOfClass:[NSDictionary class]]) continue;
                NSDictionary* pd = (NSDictionary*)p;
                const char* type = NULL;
                id t = pd[@"type"]; if ([t isKindOfClass:[NSString class]]) type = [(NSString*)t UTF8String];
                if (!type) continue;
                xpc_object_t xp = xpc_dictionary_create(NULL, NULL, 0);
                xpc_dictionary_set_string(xp, "type", type);
                if (strcmp(type, "text") == 0) {
                  id txt = pd[@"text"]; if ([txt isKindOfClass:[NSString class]]) xpc_dictionary_set_string(xp, "text", [(NSString*)txt UTF8String]);
                } else if (strcmp(type, "image") == 0) {
                  id url = pd[@"image_url"]; if ([url isKindOfClass:[NSString class]]) xpc_dictionary_set_string(xp, "image_url", [(NSString*)url UTF8String]);
                }
                xpc_array_append_value(xc, xp);
              }
              xpc_dictionary_set_value(xm, "content", xc);
            }
            xpc_array_append_value(xarr, xm);
          }
          xpc_dictionary_set_value(msg, "messages", xarr);
        }
      }
    }
    if (tools_json) {
      NSData* tdata = [NSData dataWithBytes:tools_json length:strlen(tools_json)];
      NSError* terr = nil;
      id troot = [NSJSONSerialization JSONObjectWithData:tdata options:0 error:&terr];
      if (terr == nil && [troot isKindOfClass:[NSDictionary class]]) {
        NSArray* tools = ((NSDictionary*)troot)[@"tools"];
        if ([tools isKindOfClass:[NSArray class]]) {
          xpc_object_t xarr = xpc_array_create(NULL, 0);
          for (id t in (NSArray*)tools) {
            if (![t isKindOfClass:[NSDictionary class]]) continue;
            NSString* name = ((NSDictionary*)t)[@"name"];
            id schema = ((NSDictionary*)t)[@"json_schema"];
            if (![name isKindOfClass:[NSString class]]) continue;
            xpc_object_t xt = xpc_dictionary_create(NULL, NULL, 0);
            xpc_dictionary_set_string(xt, "name", [name UTF8String]);
            if (schema) {
              NSData* sdata = [NSJSONSerialization dataWithJSONObject:schema options:0 error:nil];
              if (sdata) {
                NSString* s = [[NSString alloc] initWithData:sdata encoding:NSUTF8StringEncoding];
                if (s) xpc_dictionary_set_string(xt, "json_schema", [s UTF8String]);
              }
            }
            xpc_array_append_value(xarr, xt);
          }
          xpc_dictionary_set_value(msg, "tools", xarr);
        }
      } else {
        xpc_object_t arr = xpc_array_create(NULL, 0);
        xpc_dictionary_set_value(msg, "tools", arr);
      }
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

void* codexpc_xpc_start_from_messages(const char* service,
                                      const char* checkpoint,
                                      const char* messages_json,
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
  @autoreleasepool {
    NSString* rid = [[NSUUID UUID] UUIDString];
    strlcpy(h->req_id, [rid UTF8String], sizeof(h->req_id));
  }
  codexpc_send_create_from_messages(h, service, checkpoint, messages_json, tools_json, reasoning_json, temperature, max_tokens);
  return h;
}

static void codexpc_send_create_from_tokens(codexpc_handle* h,
                                            const char* service,
                                            const char* checkpoint,
                                            const uint32_t* tokens,
                                            size_t tokens_len,
                                            bool prime_final,
                                            const char* tools_json,
                                            const char* reasoning_json,
                                            double temperature,
                                            uint64_t max_tokens) {
  xpc_connection_t c = xpc_connection_create_mach_service(service, NULL, 0);
  h->conn = c;
  xpc_connection_set_event_handler(c, ^(xpc_object_t ev) {
    if (ev == NULL) return;
    if (xpc_get_type(ev) == XPC_TYPE_ERROR) { return; }
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
    const char* tool_name = NULL;
    const char* tool_input = NULL;
    const char* tool_output = NULL;
    if (typ && strcmp(typ, "output_item.done") == 0) {
      xpc_object_t item = xpc_dictionary_get_value(ev, "item");
      if (item && xpc_get_type(item) == XPC_TYPE_DICTIONARY) {
        code = xpc_dictionary_get_string(item, "type");
        message = xpc_dictionary_get_string(item, "status");
        tool_name = xpc_dictionary_get_string(item, "name");
        tool_input = xpc_dictionary_get_string(item, "input");
        if (!tool_input) { tool_input = xpc_dictionary_get_string(item, "arguments"); }
        tool_output = xpc_dictionary_get_string(item, "output");
        const char* call_id = xpc_dictionary_get_string(item, "call_id");
        if (call_id) { response_id = call_id; }
      }
    }
    if (h->cb) h->cb(h->ctx, typ ?: "", text, code, message, response_id, in_toks, out_toks, tot_toks, tool_name, tool_input, tool_output);
  });
  xpc_connection_resume(c);

  xpc_object_t msg = xpc_dictionary_create(NULL, NULL, 0);
  xpc_dictionary_set_string(msg, "service", service);
  xpc_dictionary_set_uint64(msg, "proto_version", 1);
  xpc_dictionary_set_string(msg, "type", "create_from_tokens");
  xpc_dictionary_set_string(msg, "req_id", h->req_id);
  xpc_dictionary_set_string(msg, "model", "gpt-oss");
  xpc_dictionary_set_string(msg, "checkpoint_path", checkpoint);
  if (tokens && tokens_len > 0) {
    xpc_object_t data = xpc_data_create(tokens, tokens_len * sizeof(uint32_t));
    xpc_dictionary_set_value(msg, "prefill_tokens", data);
  }
  xpc_dictionary_set_bool(msg, "prime_final", prime_final);
  @autoreleasepool {
    if (tools_json) {
      NSData* tdata = [NSData dataWithBytes:tools_json length:strlen(tools_json)];
      NSError* terr = nil;
      id troot = [NSJSONSerialization JSONObjectWithData:tdata options:0 error:&terr];
      if (terr == nil && [troot isKindOfClass:[NSDictionary class]]) {
        NSArray* tools = ((NSDictionary*)troot)[@"tools"];
        if ([tools isKindOfClass:[NSArray class]]) {
          xpc_object_t xarr = xpc_array_create(NULL, 0);
          for (id t in (NSArray*)tools) {
            if (![t isKindOfClass:[NSDictionary class]]) continue;
            NSString* name = ((NSDictionary*)t)[@"name"];
            id schema = ((NSDictionary*)t)[@"json_schema"];
            if (![name isKindOfClass:[NSString class]]) continue;
            xpc_object_t xt = xpc_dictionary_create(NULL, NULL, 0);
            xpc_dictionary_set_string(xt, "name", [name UTF8String]);
            if (schema) {
              NSData* sdata = [NSJSONSerialization dataWithJSONObject:schema options:0 error:nil];
              if (sdata) {
                NSString* s = [[NSString alloc] initWithData:sdata encoding:NSUTF8StringEncoding];
                if (s) xpc_dictionary_set_string(xt, "json_schema", [s UTF8String]);
              }
            }
            xpc_array_append_value(xarr, xt);
          }
          xpc_dictionary_set_value(msg, "tools", xarr);
        }
      } else {
        xpc_object_t arr = xpc_array_create(NULL, 0);
        xpc_dictionary_set_value(msg, "tools", arr);
      }
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

void* codexpc_xpc_start_from_tokens(const char* service,
                                    const char* checkpoint,
                                    const uint32_t* tokens,
                                    size_t tokens_len,
                                    bool prime_final,
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
  @autoreleasepool {
    NSString* rid = [[NSUUID UUID] UUIDString];
    strlcpy(h->req_id, [rid UTF8String], sizeof(h->req_id));
  }
  codexpc_send_create_from_tokens(h, service, checkpoint, tokens, tokens_len, prime_final, tools_json, reasoning_json, temperature, max_tokens);
  return h;
}

char* codexpc_xpc_handshake(const char* service) {
  if (!service) return NULL;
  __block char* result = NULL;
  xpc_connection_t c = xpc_connection_create_mach_service(service, NULL, 0);
  if (!c) return NULL;
  __block char req_id[64] = {0};
  @autoreleasepool {
    NSString* rid = [[NSUUID UUID] UUIDString];
    strlcpy(req_id, [rid UTF8String], sizeof(req_id));
  }
  dispatch_semaphore_t sema = dispatch_semaphore_create(0);
  xpc_connection_set_event_handler(c, ^(xpc_object_t ev) {
    if (!ev || xpc_get_type(ev) != XPC_TYPE_DICTIONARY) return;
    const char* rid = xpc_dictionary_get_string(ev, "req_id");
    if (!rid || strncmp(rid, req_id, sizeof(req_id)) != 0) return;
    const char* typ = xpc_dictionary_get_string(ev, "type");
    if (!typ) return;
    if (strcmp(typ, "handshake.ok") == 0 || strcmp(typ, "handshake.error") == 0) {
      @autoreleasepool {
        NSMutableDictionary* out = [NSMutableDictionary dictionary];
        const char* enc = xpc_dictionary_get_string(ev, "encoding_name");
        if (enc) out[@"encoding_name"] = [NSString stringWithUTF8String:enc];
        // special_tokens
        xpc_object_t st = xpc_dictionary_get_value(ev, "special_tokens");
        if (st && xpc_get_type(st) == XPC_TYPE_ARRAY) {
          NSMutableArray* arr = [NSMutableArray array];
          xpc_array_apply(st, ^bool(size_t idx, xpc_object_t v) {
            if (xpc_get_type(v) == XPC_TYPE_STRING) {
              const char* s = xpc_string_get_string_ptr(v);
              if (s) [arr addObject:[NSString stringWithUTF8String:s]];
            }
            return true;
          });
          out[@"special_tokens"] = arr;
        }
        // stop tokens
        xpc_object_t sta = xpc_dictionary_get_value(ev, "stop_tokens_for_assistant_actions");
        if (sta && xpc_get_type(sta) == XPC_TYPE_ARRAY) {
          NSMutableArray* arr = [NSMutableArray array];
          xpc_array_apply(sta, ^bool(size_t idx, xpc_object_t v) {
            if (xpc_get_type(v) == XPC_TYPE_UINT64) {
              uint64_t u = xpc_uint64_get_value(v);
              [arr addObject:@(u)];
            }
            return true;
          });
          out[@"stop_tokens_for_assistant_actions"] = arr;
        }
        NSData* data = [NSJSONSerialization dataWithJSONObject:out options:0 error:nil];
        if (data) {
          NSString* s = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
          if (s) { result = strdup([s UTF8String]); }
        }
      }
      dispatch_semaphore_signal(sema);
    }
  });
  xpc_connection_resume(c);
  xpc_object_t msg = xpc_dictionary_create(NULL, NULL, 0);
  xpc_dictionary_set_string(msg, "service", service);
  xpc_dictionary_set_uint64(msg, "proto_version", 1);
  xpc_dictionary_set_string(msg, "type", "handshake");
  xpc_dictionary_set_string(msg, "req_id", req_id);
  xpc_connection_send_message(c, msg);
  (void)dispatch_semaphore_wait(sema, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1 * NSEC_PER_SEC)));
  xpc_release(c);
  return result;
}
