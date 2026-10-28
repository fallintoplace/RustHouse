package rusthouse

import (
    "bytes"
    "context"
    "encoding/json"
    "fmt"
    "io"
    "net/http"
    "net/url"
    "strings"
    "time"
)

// Client uses RustHouse's experimental, read-only HTTP API. It is not a
// ClickHouse wire-protocol driver. No automatic retries are performed.
type Client struct {
    Origin string
    HTTP *http.Client
}

// New accepts loopback origins only. The HTTP server has no TLS or authentication.
func New(origin string) (*Client, error) {
    u, err := url.Parse(origin)
    if err != nil { return nil, err }
    if u.Scheme != "http" || (u.Hostname() != "localhost" && u.Hostname() != "127.0.0.1") ||
        u.User != nil || (u.Path != "" && u.Path != "/") || u.RawQuery != "" || u.Fragment != "" {
        return nil, fmt.Errorf("RustHouse requires a loopback HTTP origin")
    }
    if u.Port() == "" { return nil, fmt.Errorf("RustHouse requires an explicit port") }
    return &Client{Origin: strings.TrimSuffix(origin, "/"), HTTP: &http.Client{Timeout: 5*time.Second}}, nil
}

type HTTPError struct {
    Status int
    Body string
}
func (e *HTTPError) Error() string {
    return fmt.Sprintf("RustHouse HTTP %d: %s", e.Status, e.Body)
}

const maxResponseBytes = 16 << 20

func (c *Client) request(ctx context.Context, path, sql string, post bool) ([]byte, error) {
    if post && (len(sql) == 0 || len(sql) > 4096) {
        return nil, fmt.Errorf("SQL must contain 1..4096 UTF-8 bytes")
    }
    var body io.Reader
    method := http.MethodGet
    if post { method = http.MethodPost; body = bytes.NewBufferString(sql) }
    req, err := http.NewRequestWithContext(ctx, method, c.Origin + path, body)
    if err != nil { return nil, err }
    if post { req.Header.Set("Content-Type", "text/plain; charset=utf-8") }
    resp, err := c.HTTP.Do(req)
    if err != nil { return nil, err }
    defer resp.Body.Close()
    payload, err := io.ReadAll(io.LimitReader(resp.Body, maxResponseBytes+1))
    if err != nil { return nil, err }
    if len(payload) > maxResponseBytes { return nil, fmt.Errorf("RustHouse response exceeds 16 MiB") }
    if resp.StatusCode < 200 || resp.StatusCode >= 300 {
        message := string(payload)
        if len(message) > 200 { message = message[:200] }
        return nil, &HTTPError{Status: resp.StatusCode, Body: message}
    }
    return payload, nil
}

// QueryJSON returns a JSON array with RustHouse's exact string numerics intact.
func (c *Client) QueryJSON(ctx context.Context, sql string) ([]map[string]interface{}, error) {
    payload, err := c.request(ctx, "/?format=json", sql, true)
    if err != nil { return nil, err }
    var rows []map[string]interface{}
    if err := json.Unmarshal(payload, &rows); err != nil { return nil, err }
    if rows == nil { return nil, fmt.Errorf("expected a JSON row array") }
    return rows, nil
}

func (c *Client) Capabilities(ctx context.Context) (map[string]interface{}, error) {
    payload, err := c.request(ctx, "/capabilities", "", false)
    if err != nil { return nil, err }
    var info map[string]interface{}
    err = json.Unmarshal(payload, &info)
    return info, err
}

func (c *Client) Schema(ctx context.Context, table string) (map[string]interface{}, error) {
    path := "/schema"
    if table != "" {
        if len(table) > 128 { return nil, fmt.Errorf("table name exceeds 128 bytes") }
        path += "?table=" + url.QueryEscape(table)
    }
    payload, err := c.request(ctx, path, "", false)
    if err != nil { return nil, err }
    var info map[string]interface{}
    err = json.Unmarshal(payload, &info)
    return info, err
}
