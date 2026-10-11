package app

import (
	"bufio"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"maps"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"

	"qbutt.org/search-engine/internal/indexer/cardigann"
)

type instance struct {
	client   *http.Client
	engine   *cardigann.Engine
	settings map[string]string
}

type browserSession struct {
	Cookie    string
	UserAgent string
}

type App struct {
	dir         string
	mu          sync.Mutex
	preferences preferences
	sources     map[string]Source
	instances   map[string]*instance
	sessions    map[string]browserSession
	requests    map[int]context.CancelFunc
	cache       map[string]cachedSearch
	variants    map[string]storedRelease
	releaseMu   sync.Mutex
	writeMu     sync.Mutex
	writer      io.Writer
	updateMu    sync.Mutex
}

type cachedSearch struct {
	works []Work
	at    time.Time
}

func New(dir string) (*App, error) {
	if err := os.MkdirAll(dir, 0700); err != nil {
		return nil, err
	}
	p, err := loadPreferences(dir)
	if err != nil {
		return nil, err
	}
	releases, err := loadReleases(dir)
	if err != nil {
		return nil, err
	}
	a := &App{dir: dir, preferences: p, instances: map[string]*instance{}, sessions: map[string]browserSession{}, requests: map[int]context.CancelFunc{}, cache: map[string]cachedSearch{}, variants: releases}
	if err := recoverDefinitions(dir); err != nil {
		return nil, err
	}
	if err := a.loadCatalog(); err != nil {
		return nil, err
	}
	return a, nil
}

func (a *App) Serve(ctx context.Context, in io.Reader, out io.Writer) error {
	ctx, cancelServe := context.WithCancel(ctx)
	a.writer = out
	var wg sync.WaitGroup
	defer func() {
		cancelServe()
		a.mu.Lock()
		for _, cancel := range a.requests {
			cancel()
		}
		a.mu.Unlock()
		wg.Wait()
	}()
	// Network refresh never delays the first catalogue or first search.
	wg.Go(func() { _ = a.update(ctx, false) })
	scanner := bufio.NewScanner(in)
	scanner.Buffer(make([]byte, 64<<10), 16<<20)
	for scanner.Scan() {
		var request Request
		if err := json.Unmarshal(scanner.Bytes(), &request); err != nil {
			a.emit(0, "error", map[string]string{"message": "Invalid helper request"})
			continue
		}
		if request.ID <= 0 {
			a.emit(request.ID, "error", map[string]string{"message": "A positive request ID is required"})
			continue
		}
		requestContext, cancel := context.WithCancel(ctx)
		a.mu.Lock()
		_, duplicate := a.requests[request.ID]
		if !duplicate {
			a.requests[request.ID] = cancel
		}
		a.mu.Unlock()
		if duplicate {
			cancel()
			a.emit(request.ID, "error", map[string]string{"message": "Request ID is already active"})
			continue
		}
		wg.Go(func() {
			defer cancel()
			defer func() { a.mu.Lock(); delete(a.requests, request.ID); a.mu.Unlock() }()
			if err := a.handle(requestContext, request); err != nil {
				a.emit(request.ID, "error", map[string]string{"message": safeError(err)})
			} else {
				a.emit(request.ID, "complete", nil)
			}
		})
	}
	return scanner.Err()
}

func (a *App) emit(id int, event string, data any) {
	a.writeMu.Lock()
	defer a.writeMu.Unlock()
	_ = json.NewEncoder(a.writer).Encode(Event{ID: id, Event: event, Data: data})
}

func (a *App) handle(ctx context.Context, r Request) error {
	if r.Method == "search" || r.Method == "inspect" || r.Method == "resolve" {
		if err := a.browserSessions(r.Params.Cookies, r.Params.UserAgents); err != nil {
			return err
		}
	}
	switch r.Method {
	case "sources":
		a.emit(r.ID, "sources", a.catalog())
	case "search":
		return a.search(ctx, r)
	case "configureSource":
		if err := a.configure(r.Params); err != nil {
			return err
		}
		a.emit(r.ID, "sources", a.catalog())
	case "update":
		if err := a.update(ctx, true); err != nil {
			return err
		}
		a.emit(r.ID, "sources", a.catalog())
	case "discover":
		return a.discover(ctx, r)
	case "inspect":
		details, err := a.inspect(ctx, r.Params.Variant)
		if err != nil {
			return err
		}
		a.emit(r.ID, "details", a.publicWork(details))
	case "resolve":
		resolved, err := a.resolve(ctx, r.Params.Variant)
		if err != nil {
			return err
		}
		a.emit(r.ID, "resolved", resolved)
	case "cancel":
		a.mu.Lock()
		cancel := a.requests[r.Params.RequestID]
		a.mu.Unlock()
		if cancel != nil {
			cancel()
		}
	default:
		return errors.New("unknown helper method")
	}
	return nil
}

func (a *App) sourceInstance(id string) (Source, *instance, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	source, ok := a.sources[id]
	if !ok {
		return Source{}, nil, errors.New("unknown source")
	}
	if cached := a.instances[id]; cached != nil {
		return source, cached, nil
	}
	settings := maps.Clone(a.preferences.Sources[id].Settings)
	if settings == nil {
		settings = map[string]string{}
	}
	if settings["useragent"] == "" {
		settings["useragent"] = settings["user_agent"]
	}
	if settings["useragent"] == "" {
		settings["useragent"] = a.sessions[id].UserAgent
	}
	settings["user_agent"] = settings["useragent"]
	if settings["cookie"] == "" && settings["password"] == "" {
		settings["cookie"] = a.sessions[id].Cookie
	}
	if settings["cookie"] != "" && settings["solver_type"] == "" {
		settings["solver_type"] = "manual_cookie"
	}
	client, err := newClient(settings, sourceInterval(source))
	if err != nil {
		return source, nil, err
	}
	i := &instance{client: client, settings: settings}
	if source.definition != nil {
		opts := []cardigann.Option{cardigann.WithDoer(client), cardigann.WithConfig(settings), cardigann.SolverOption(settings)}
		if base := settings["baseUrl"]; base != "" {
			opts = append(opts, cardigann.WithBaseURL(base))
		}
		i.engine, err = cardigann.NewEngine(source.definition, opts...)
		if err != nil {
			return source, nil, err
		}
	}
	a.instances[id] = i
	return source, i, nil
}

func (a *App) browserSessions(cookies, userAgents map[string]string) error {
	a.mu.Lock()
	defer a.mu.Unlock()
	for id, cookie := range cookies {
		if _, ok := a.sources[id]; !ok || len(cookie) > 64<<10 || strings.ContainsAny(cookie, "\r\n") {
			return errors.New("invalid browser session")
		}
	}
	for id, userAgent := range userAgents {
		if _, ok := a.sources[id]; !ok || len(userAgent) > 8192 || strings.ContainsAny(userAgent, "\r\n") {
			return errors.New("invalid browser user agent")
		}
	}
	updated := maps.Clone(a.sessions)
	for id, cookie := range cookies {
		session := updated[id]
		session.Cookie = cookie
		updated[id] = session
	}
	for id, userAgent := range userAgents {
		session := updated[id]
		session.UserAgent = userAgent
		updated[id] = session
	}
	for id, session := range updated {
		if a.sessions[id] == session {
			continue
		}
		if session == (browserSession{}) {
			delete(a.sessions, id)
		} else {
			a.sessions[id] = session
		}
		delete(a.instances, id)
		clear(a.cache)
	}
	return nil
}

func torrentResolved(variant Variant, data []byte, magnet string) (Resolved, error) {
	if magnet != "" {
		if !validMagnet(magnet) {
			return Resolved{}, errors.New("source returned an invalid magnet")
		}
		if variant.InfoHash != "" && !strings.EqualFold(variant.InfoHash, magnetHash(magnet)) {
			return Resolved{}, errors.New("the source returned a different release")
		}
		return Resolved{Kind: "torrent", URL: magnet, Title: variant.Title}, nil
	}
	if len(data) == 0 || len(data) > 8<<20 || data[0] != 'd' || !validTorrent(data) {
		return Resolved{}, errors.New("source did not return valid torrent metadata")
	}
	if variant.InfoHash != "" && !strings.EqualFold(variant.InfoHash, torrentHash(data, len(variant.InfoHash) == 64)) {
		return Resolved{}, errors.New("the source returned a different release")
	}
	return Resolved{Kind: "torrent", TorrentData: base64.StdEncoding.EncodeToString(data), Title: variant.Title}, nil
}
