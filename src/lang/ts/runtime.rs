//! The fixed TypeScript runtime the generated client is built around — config
//! types, the `request`/`send`/`validate`/`toQuery` helpers — assembled by
//! [`super::client`].

pub(super) const CONFIG_BLOCK: &str = r#"
export interface ApiConfig {
  /** API origin, e.g. PUBLIC_SERVICE_URL on Railway. Defaults to same-origin. */
  baseUrl?: string;
  /** fetch to use — pass `event.fetch` on the server (SvelteKit / Workers). */
  fetch?: typeof fetch;
  /** Headers sent on every request, e.g. `{ cookie }` forwarded from the
   *  incoming request during SSR (the browser sends the cookie automatically). */
  headers?: Record<string, string>;
}

export interface ApiError {
  /** HTTP status, 0 for a transport error, or 422 for request validation. */
  status: number;
  message: string;
  /** Field path -> message, present when `status` is 422 (body validation). */
  fields?: Record<string, string>;
}
"#;

pub(super) const REQUEST_HELPER: &str = r#"  const base = (config.baseUrl ?? "").replace(/\/$/, "");
  const fetchImpl = config.fetch ?? fetch;

  async function request<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<Result<T, ApiError>> {
    const headers: Record<string, string> = { ...config.headers };

    if (body !== undefined) {
      headers["Content-Type"] = "application/json";
    }

    const response = await result(() =>
      fetchImpl(base + path, {
        method,
        credentials: "include",
        headers,
        body: body !== undefined ? JSON.stringify(body) : undefined,
      }),
    );

    if (response.err) {
      const cause = response.error;
      return err({
        status: 0,
        message: `${method} ${path}: ${cause instanceof Error ? cause.message : String(cause)}`,
      });
    }

    const res = response.value;

    if (!res.ok) {
      const body = await res.text().catch(() => "");
      let parsed: { error?: string; fields?: Record<string, string> } | undefined;
      try {
        parsed = body ? JSON.parse(body) : undefined;
      } catch {
        parsed = undefined;
      }
      return err({
        status: res.status,
        message: parsed?.error ?? `${method} ${path} failed (${res.status})${body ? `: ${body}` : ""}`,
        fields: parsed?.fields,
      });
    }

    const text = await res.text();

    return ok(text ? (JSON.parse(text) as T) : (undefined as T));
  }
"#;

pub(super) const VALIDATE_HELPER: &str = r#"
  function validate(schema: v.GenericSchema, body: unknown): Result<unknown, ApiError> {
    const parsed = v.safeParse(schema, body);

    if (parsed.success) {
      return ok(parsed.output);
    }

    const fields: Record<string, string> = {};
    for (const issue of parsed.issues) {
      fields[v.getDotPath(issue) ?? ""] = issue.message;
    }

    return err({
      status: 422,
      message: "Request validation failed",
      fields,
    });
  }
"#;

pub(super) const SEND_HELPER: &str = r#"
  function send<T>(
    method: string,
    path: string,
    schema: v.GenericSchema,
    body: unknown,
  ): Promise<Result<T, ApiError>> {
    const valid = validate(schema, body);

    if (valid.err) {
      return Promise.resolve(valid);
    }

    return request<T>(method, path, valid.value);
  }
"#;

pub(super) const QUERY_HELPER: &str = r#"
  function toQuery(params: Record<string, unknown>): string {
    const search = new URLSearchParams();
    for (const [key, value] of Object.entries(params)) {
      if (value !== undefined && value !== null) search.set(key, String(value));
    }
    const query = search.toString();
    return query ? `?${query}` : "";
  }
"#;
