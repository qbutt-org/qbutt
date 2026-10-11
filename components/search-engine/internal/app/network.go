package app

import (
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/cookiejar"
	"net/netip"
	"net/url"
	"strings"
	"sync"
	"time"

	"golang.org/x/net/publicsuffix"
	apphttp "qbutt.org/search-engine/internal/http"
	"qbutt.org/search-engine/internal/providers"
)

const maxResponseBytes = 32 << 20

type sourceTransport struct {
	base      *http.Transport
	interval  time.Duration
	mu        sync.Mutex
	next      time.Time
	userAgent string
}

func publicAddress(ip netip.Addr) bool {
	ip = ip.Unmap()
	return ip.IsGlobalUnicast() && !ip.IsPrivate() && !ip.IsLoopback() &&
		!ip.IsLinkLocalUnicast() && !ip.IsLinkLocalMulticast() &&
		!netip.MustParsePrefix("100.64.0.0/10").Contains(ip) &&
		!netip.MustParsePrefix("198.18.0.0/15").Contains(ip) &&
		!netip.MustParsePrefix("192.0.0.0/24").Contains(ip)
}

func resolvePublic(ctx context.Context, host string) ([]netip.Addr, error) {
	addresses, err := net.DefaultResolver.LookupNetIP(ctx, "ip", host)
	if err != nil {
		return nil, errors.New("site address could not be resolved")
	}
	if len(addresses) == 0 {
		return nil, errors.New("site has no public address")
	}
	for _, ip := range addresses {
		if !publicAddress(ip) {
			return nil, errors.New("local and private network addresses are not allowed")
		}
	}
	return addresses, nil
}

func checkURL(raw string) (*url.URL, error) {
	u, err := url.Parse(raw)
	if err != nil || u.Hostname() == "" || u.User != nil || (u.Scheme != "https" && u.Scheme != "http") {
		return nil, errors.New("a public HTTP or HTTPS site address is required")
	}
	if u.Fragment != "" {
		u.Fragment = ""
	}
	return u, nil
}

func newClient(settings map[string]string, interval time.Duration) (*http.Client, error) {
	jar, err := cookiejar.New(&cookiejar.Options{PublicSuffixList: publicsuffix.List})
	if err != nil {
		return nil, err
	}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.Proxy = nil
	transport.MaxConnsPerHost = 2
	transport.ResponseHeaderTimeout = 15 * time.Second
	transport.TLSHandshakeTimeout = 10 * time.Second
	transport.MaxResponseHeaderBytes = 1 << 20
	proxyHost := ""
	if raw := settings["proxyUrl"]; raw != "" {
		proxyURL, err := url.Parse(raw)
		if err != nil || proxyURL.Hostname() == "" || (proxyURL.Scheme != "http" && proxyURL.Scheme != "socks5" && proxyURL.Scheme != "socks5h") {
			return nil, errors.New("proxy address must use HTTP or SOCKS5")
		}
		proxyHost = proxyURL.Hostname()
		transport.Proxy = http.ProxyURL(proxyURL)
	}
	transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		host, port, err := net.SplitHostPort(address)
		if err != nil {
			return nil, errors.New("invalid network address")
		}
		dialer := net.Dialer{Timeout: 10 * time.Second, KeepAlive: 30 * time.Second}
		if proxyHost != "" && strings.EqualFold(host, proxyHost) {
			return dialer.DialContext(ctx, network, address)
		}
		addresses, err := resolvePublic(ctx, host)
		if err != nil {
			return nil, err
		}
		for _, ip := range addresses {
			conn, err := dialer.DialContext(ctx, network, net.JoinHostPort(ip.String(), port))
			if err == nil {
				return conn, nil
			}
		}
		return nil, errors.New("site connection failed")
	}
	if interval < time.Second {
		interval = time.Second
	}
	if interval > 10*time.Second {
		interval = 10 * time.Second
	}
	return &http.Client{
		Transport: &sourceTransport{base: transport, interval: interval, userAgent: settings["useragent"]},
		Jar:       jar,
		Timeout:   30 * time.Second,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if err := apphttp.RedirectPolicy(req, via); err != nil {
				return err
			}
			if len(via) >= 5 {
				return errors.New("site redirected too many times")
			}
			if len(via) > 0 && via[0].URL.Scheme == "https" && req.URL.Scheme == "http" {
				return errors.New("site redirected to an insecure connection")
			}
			if len(via) > 0 && req.URL.Hostname() != via[0].URL.Hostname() {
				req.Header.Del("Authorization")
				req.Header.Del("Cookie")
			}
			return nil
		},
	}, nil
}

func (t *sourceTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	if _, err := checkURL(req.URL.String()); err != nil {
		return nil, err
	}
	// Validate the ultimate origin even when an explicitly configured proxy
	// resolves the destination. Direct dialing independently pins the checked IP.
	if _, err := resolvePublic(req.Context(), req.URL.Hostname()); err != nil {
		return nil, err
	}
	t.mu.Lock()
	when := t.next
	if when.Before(time.Now()) {
		when = time.Now()
	}
	t.next = when.Add(t.interval)
	t.mu.Unlock()
	if delay := time.Until(when); delay > 0 {
		timer := time.NewTimer(delay)
		defer timer.Stop()
		select {
		case <-timer.C:
		case <-req.Context().Done():
			return nil, req.Context().Err()
		}
	}
	clone := req.Clone(req.Context())
	if clone.Header.Get("User-Agent") == "" {
		agent := t.userAgent
		if agent == "" {
			agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 qbutt/1"
		}
		clone.Header.Set("User-Agent", agent)
	}
	if clone.Header.Get("Accept-Language") == "" {
		clone.Header.Set("Accept-Language", "ru,en;q=0.8")
	}
	response, err := t.base.RoundTrip(clone)
	if err != nil {
		return nil, errors.New("site connection failed")
	}
	if response.ContentLength > maxResponseBytes {
		response.Body.Close()
		return nil, errors.New("site response exceeded the size limit")
	}
	response.Body = &boundedBody{ReadCloser: response.Body, remaining: maxResponseBytes}
	return response, nil
}

type boundedBody struct {
	io.ReadCloser
	remaining int64
}

func (b *boundedBody) Read(p []byte) (int, error) {
	if b.remaining <= 0 {
		return 0, errors.New("site response exceeded the size limit")
	}
	if int64(len(p)) > b.remaining {
		p = p[:b.remaining]
	}
	n, err := b.ReadCloser.Read(p)
	b.remaining -= int64(n)
	return n, err
}

func safeError(err error) string {
	if errors.Is(err, providers.ErrLoginRequired) {
		return "Sign in to this source"
	}
	if errors.Is(err, providers.ErrChallenge) {
		return "Open this source in your browser to complete its verification"
	}
	if errors.Is(err, providers.ErrNoDownload) {
		return "This source has no direct torrent download; open the original page"
	}
	if errors.Is(err, providers.ErrChooseVariant) {
		return "Choose a release variant"
	}
	if errors.Is(err, context.Canceled) {
		return "Search cancelled"
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return "The site did not answer in time"
	}
	// Upstream parser failures may quote tracker-authored body text. Expose a
	// classified diagnosis, never a raw URL, response, cookie or account value.
	s := strings.ToLower(err.Error())
	switch {
	case strings.Contains(s, "captcha"), strings.Contains(s, "solver"), strings.Contains(s, "cloudflare"), strings.Contains(s, "http 403"):
		return "The site requires a browser check or a refreshed session"
	case strings.Contains(s, "login"), strings.Contains(s, "unauthorized"), strings.Contains(s, "http 401"):
		return "Sign in to this source"
	case strings.Contains(s, "http 429"), strings.Contains(s, "rate limit"):
		return "The site is limiting requests; try again later"
	case strings.Contains(s, "http 451"):
		return "The site is unavailable on this network"
	case strings.Contains(s, "parsing"), strings.Contains(s, "template"), strings.Contains(s, "selector"):
		return "The source definition needs an update"
	case strings.Contains(s, "private network"), strings.Contains(s, "local and"):
		return "Local and private network addresses are not allowed"
	default:
		return "The source could not complete the request"
	}
}
