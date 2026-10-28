package rusthouse

import (
    "context"
    "errors"
    "io"
    "net/http"
    "net/http/httptest"
    "testing"
)
func TestLoopbackOnly(t *testing.T) {
    for _, origin := range []string{"https://127.0.0.1:8123", "http://example.org:8123", "http://127.0.0.1:8123/?token=x"} {
        if _, err := New(origin); err == nil { t.Errorf("accepted unsafe origin: %s", origin) }
    }
}
func TestQueryPreservesWideInteger(t *testing.T) {
    server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
        if r.Method != "POST" || r.URL.RequestURI() != "/?format=json" { t.Errorf("request: %s %s", r.Method, r.URL) }
        got, _ := io.ReadAll(r.Body)
        if string(got) != "SELECT 1" { t.Errorf("SQL: %s", got) }
        w.Write([]byte("[{\"n\":\"18446744073709551615\"}]"))
    }))
    defer server.Close()
    client, err := New(server.URL)
    if err != nil { t.Fatal(err) }
    rows, err := client.QueryJSON(context.Background(), "SELECT 1")
    if err != nil { t.Fatal(err) }
    if rows[0]["n"] != "18446744073709551615" { t.Fatal(rows) }
}
func TestReturnsHTTPError(t *testing.T) {
    server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
        http.Error(w, "invalid query", http.StatusBadRequest)
    }))
    defer server.Close()
    client, err := New(server.URL)
    if err != nil { t.Fatal(err) }
    _, err = client.QueryJSON(context.Background(), "SELECT bad")
    var httpError *HTTPError
    if !errors.As(err, &httpError) || httpError.Status != 400 { t.Fatalf("expected HTTP 400, got %v", err) }
}
