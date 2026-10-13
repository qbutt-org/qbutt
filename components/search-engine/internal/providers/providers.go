package providers

import (
	"bytes"
	"context"
	"encoding/base32"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/cookiejar"
	"net/url"
	"strings"
	"time"

	"github.com/PuerkitoBio/goquery"
	"golang.org/x/net/html/charset"
)

type Source struct {
	ID            string
	Name          string
	URL           string
	Categories    []string
	RequiresLogin bool
}

type Record struct {
	Title       string
	URL         string
	DownloadURL string
	Magnet      string
	InfoHash    string
	Description string
	Cover       string
	Size        int64
	Seeders     int64
	PublishedAt string
	Variants    []Record
}

var (
	ErrLoginRequired = errors.New("sign in to the source")
	ErrChallenge     = errors.New("complete the source's browser verification")
	ErrNoDownload    = errors.New("this source has no direct torrent download; open the original page")
	ErrChooseVariant = errors.New("choose a release variant")
)

func Sources() []Source {
	return []Source{
		{
			ID:            "rutracker",
			Name:          "RuTracker",
			URL:           "https://rutracker.org",
			Categories:    []string{"all", "games", "movies", "tv", "software", "books", "anime"},
			RequiresLogin: true,
		},
	}
}

func Search(ctx context.Context, id, query string, settings map[string]string, client *http.Client) ([]Record, error) {
	s, err := newSession(ctx, id, settings, client)
	if err != nil {
		return nil, err
	}
	if strings.TrimSpace(query) == "" {
		return nil, nil
	}
	return s.searchRuTracker(query)
}

func Inspect(ctx context.Context, id, address string, settings map[string]string, client *http.Client) (Record, error) {
	s, err := newSession(ctx, id, settings, client)
	if err != nil {
		return Record{}, err
	}
	return s.inspect(address)
}

func Resolve(ctx context.Context, id, address string, settings map[string]string, client *http.Client) ([]byte, string, error) {
	s, err := newSession(ctx, id, settings, client)
	if err != nil {
		return nil, "", err
	}
	if strings.HasPrefix(address, "magnet:") {
		if magnetHash(address) == "" {
			return nil, "", ErrNoDownload
		}
		return nil, address, nil
	}
	return s.resolve(address, 0)
}

type session struct {
	ctx      context.Context
	source   Source
	settings map[string]string
	client   *http.Client
}

func newSession(ctx context.Context, id string, settings map[string]string, client *http.Client) (*session, error) {
	var source Source
	for _, candidate := range Sources() {
		if candidate.ID == id {
			source = candidate
			break
		}
	}
	if source.ID == "" {
		return nil, fmt.Errorf("unknown source %q", id)
	}
	if client == nil {
		client = &http.Client{Timeout: 30 * time.Second}
	}
	if client.Jar == nil {
		copy := *client
		copy.Jar, _ = cookiejar.New(nil)
		client = &copy
	}
	if cookie := settings["cookie"]; cookie != "" {
		base, _ := url.Parse(source.URL)
		request := &http.Request{Header: http.Header{"Cookie": {cookie}}}
		cookies := request.Cookies()
		for _, value := range cookies {
			value.Secure = true
			value.HttpOnly = true
		}
		client.Jar.SetCookies(base, cookies)
	}
	return &session{ctx: ctx, source: source, settings: settings, client: client}, nil
}

func (s *session) inspect(address string) (Record, error) {
	u, err := url.Parse(address)
	if err != nil || !sameHost(address, s.source.URL) {
		return Record{}, errors.New("release page does not belong to the selected source")
	}
	return s.inspectRuTracker(u)
}

func (s *session) request(method, address, referer, body string) ([]byte, string, error) {
	u, err := url.Parse(address)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Host == "" || u.User != nil {
		return nil, "", errors.New("invalid source URL")
	}
	req, err := http.NewRequestWithContext(s.ctx, method, address, strings.NewReader(body))
	if err != nil {
		return nil, "", err
	}
	userAgent := s.settings["user_agent"]
	if userAgent == "" {
		userAgent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36"
	}
	req.Header.Set("User-Agent", userAgent)
	req.Header.Set("Accept-Language", "ru,en;q=0.8")
	if referer != "" {
		req.Header.Set("Referer", referer)
	}
	if method == http.MethodPost {
		req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	}
	response, err := s.client.Do(req)
	if err != nil {
		return nil, "", err
	}
	defer response.Body.Close()

	const maxResponseBytes = 32 << 20
	data, err := io.ReadAll(io.LimitReader(response.Body, maxResponseBytes+1))
	if err != nil {
		return nil, "", err
	}
	if len(data) > maxResponseBytes {
		return nil, "", errors.New("source response exceeds 32 MiB")
	}
	lower := strings.ToLower(string(data[:min(len(data), 16384)]))
	if strings.Contains(lower, "<title>just a moment") || strings.Contains(lower, "cf-chl-widget") || strings.Contains(lower, "_cf_chl_opt") {
		return nil, "", ErrChallenge
	}
	if response.StatusCode == http.StatusUnauthorized {
		return nil, "", ErrLoginRequired
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, "", fmt.Errorf("source returned HTTP %d", response.StatusCode)
	}
	return data, response.Header.Get("Content-Type"), nil
}

func (s *session) document(address, referer string) (*goquery.Document, error) {
	data, contentType, err := s.request(http.MethodGet, address, referer, "")
	if err != nil {
		return nil, err
	}
	return parseDocument(data, contentType)
}

func parseDocument(data []byte, contentType string) (*goquery.Document, error) {
	reader, err := charset.NewReader(bytes.NewReader(data), contentType)
	if err != nil {
		return nil, err
	}
	return goquery.NewDocumentFromReader(reader)
}

func sameHost(address, base string) bool {
	u, err := url.Parse(address)
	b, baseErr := url.Parse(base)
	return err == nil && baseErr == nil && strings.EqualFold(u.Host, b.Host)
}

func absoluteURL(base, address string) string {
	u, err := url.Parse(strings.TrimSpace(address))
	if err != nil {
		return ""
	}
	b, err := url.Parse(base)
	if err != nil {
		return ""
	}
	u = b.ResolveReference(u)
	if u.User != nil || (u.Scheme != "http" && u.Scheme != "https" && u.Scheme != "magnet") {
		return ""
	}
	return u.String()
}

func plainText(selection *goquery.Selection) string {
	clone := selection.Clone()
	clone.Find("script, style, iframe, noscript").Remove()
	clone.Find("br").ReplaceWithHtml("\n")
	clone.Find("p, li, div, h2, h3, h4").PrependHtml("\n")
	var lines []string
	for _, line := range strings.Split(clone.Text(), "\n") {
		if line = strings.Join(strings.Fields(line), " "); line != "" {
			lines = append(lines, line)
		}
	}
	text := []rune(strings.Join(lines, "\n"))
	if len(text) > 16000 {
		text = text[:16000]
	}
	return string(text)
}

func imageURL(selection *goquery.Selection, base string) string {
	image := selection.Find("img").First()
	for _, attribute := range []string{"data-src", "src"} {
		if value, ok := image.Attr(attribute); ok {
			return absoluteURL(base, value)
		}
	}
	return ""
}

func magnetHash(magnet string) string {
	u, err := url.Parse(magnet)
	if err != nil || u.Scheme != "magnet" {
		return ""
	}
	for _, xt := range u.Query()["xt"] {
		if !strings.HasPrefix(strings.ToLower(xt), "urn:btih:") {
			continue
		}
		hash := strings.TrimPrefix(strings.ToLower(xt), "urn:btih:")
		if len(hash) == 40 {
			if _, err := hex.DecodeString(hash); err == nil {
				return hash
			}
		}
		if len(hash) == 32 {
			if decoded, err := base32.StdEncoding.DecodeString(strings.ToUpper(hash)); err == nil && len(decoded) == 20 {
				return hex.EncodeToString(decoded)
			}
		}
	}
	return ""
}

func attachVariants(record *Record, variants []Record) {
	seen := make(map[string]bool)
	for _, variant := range variants {
		key := variant.InfoHash
		if key == "" {
			key = variant.DownloadURL
		}
		if key == "" || seen[key] {
			continue
		}
		seen[key] = true
		record.Variants = append(record.Variants, variant)
	}
	if len(record.Variants) == 1 {
		variant := record.Variants[0]
		record.DownloadURL = variant.DownloadURL
		record.Magnet = variant.Magnet
		record.InfoHash = variant.InfoHash
	}
}
