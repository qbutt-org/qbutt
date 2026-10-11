package app

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"maps"
	"net/url"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"qbutt.org/search-engine/internal/indexer/cardigann"
	"qbutt.org/search-engine/internal/indexer/cardigann/loader"
	"qbutt.org/search-engine/internal/providers"
)

func identity(parts ...string) string {
	digest := sha256.Sum256([]byte(strings.Join(parts, "\x00")))
	return hex.EncodeToString(digest[:16])
}

func (a *App) search(ctx context.Context, r Request) error {
	p := r.Params
	p.Query = strings.TrimSpace(p.Query)
	if p.Query == "" || len(p.Query) > 1024 {
		return errors.New("enter a search query")
	}
	if p.Category == "" {
		p.Category = "all"
	}
	var selected, secondary []Source
	a.mu.Lock()
	configured := maps.Clone(a.preferences.Sources)
	a.mu.Unlock()
	for _, source := range a.catalog() {
		if !matchesCategory(source, p.Category) {
			continue
		}
		if len(p.Sources) > 0 {
			if slices.Contains(p.Sources, source.ID) {
				selected = append(selected, source)
			}
		} else if source.Enabled {
			selected = append(selected, source)
		} else if !source.RequiresLogin && configured[source.ID].Enabled == nil {
			secondary = append(secondary, source)
		}
	}
	var results atomic.Int64
	run := func(sources []Source) {
		var wg sync.WaitGroup
		sem := make(chan struct{}, 6)
		for _, source := range sources {
			if ctx.Err() != nil {
				break
			}
			wg.Go(func() {
				select {
				case sem <- struct{}{}:
				case <-ctx.Done():
					return
				}
				defer func() { <-sem }()
				a.emit(r.ID, "source", map[string]any{"sourceId": source.ID, "status": "searching"})
				sourceCtx, cancel := context.WithTimeout(ctx, 45*time.Second)
				defer cancel()
				works, err := a.searchSource(sourceCtx, source.ID, p.Query, p.Category)
				if err != nil {
					status := "error"
					if errors.Is(err, providers.ErrLoginRequired) || safeError(err) == "Sign in to this source" {
						status = "login_required"
					}
					a.emit(r.ID, "source", map[string]any{"sourceId": source.ID, "status": status, "message": safeError(err)})
					return
				}
				for _, work := range works {
					if ctx.Err() != nil {
						return
					}
					a.remember(work)
					a.emit(r.ID, "result", a.publicWork(work))
				}
				results.Add(int64(len(works)))
				if err := a.saveReleases(); err != nil {
					a.emit(r.ID, "source", map[string]any{"sourceId": source.ID, "status": "error", "message": "Release history could not be saved"})
					return
				}
				a.emit(r.ID, "source", map[string]any{"sourceId": source.ID, "status": "complete", "count": len(works)})
			})
		}
		wg.Wait()
	}
	run(selected)
	if ctx.Err() != nil {
		return ctx.Err()
	}
	// Extend a thin search without fanning one query out to the whole catalogue.
	// Explicit source selection is exact; expanded mode searches all enabled
	// sources and a bounded public discovery cohort for the chosen category.
	if len(p.Sources) == 0 && (results.Load() < 10 || p.Expanded) {
		limit := 12
		if p.Expanded {
			limit = 36
		}
		run(secondary[:min(limit, len(secondary))])
	}
	return ctx.Err()
}

func (a *App) searchSource(ctx context.Context, id, query, category string) ([]Work, error) {
	key := id + "\x00" + query + "\x00" + category
	a.mu.Lock()
	cached, ok := a.cache[key]
	a.mu.Unlock()
	if ok && time.Since(cached.at) < 5*time.Minute {
		return cached.works, nil
	}
	source, i, err := a.sourceInstance(id)
	if err != nil {
		return nil, err
	}
	var works []Work
	if i.engine != nil {
		releases, err := searchReleases(ctx, source, i, i.engine, query, category)
		if len(releases) == 0 && (err == nil || isParserError(err)) {
			// A selector change may silently return zero rows. Revert only after
			// the previous definition proves it can still find this actual query.
			if data, previous := a.previousSource(id); previous != nil {
				opts := []cardigann.Option{cardigann.WithDoer(i.client), cardigann.WithConfig(i.settings), cardigann.SolverOption(i.settings)}
				if base := i.settings["baseUrl"]; base != "" {
					opts = append(opts, cardigann.WithBaseURL(base))
				}
				if engine, buildErr := cardigann.NewEngine(previous, opts...); buildErr == nil {
					old, oldErr := searchReleases(ctx, source, i, engine, query, category)
					if oldErr == nil && len(old) > 0 {
						a.restoreSource(id, source.definition, previous, data)
						releases, err = old, nil
						source.definition = previous
					}
				}
			}
		}
		if err != nil {
			return nil, err
		}
		for _, release := range releases[:min(len(releases), 500)] {
			if releaseMatchesCategory(release.Categories, category) {
				works = append(works, releaseWork(source, release, category))
			}
		}
	} else if strings.HasPrefix(id, "custom-") {
		works, err = a.searchCustom(ctx, source, i, query)
		if err != nil {
			return nil, err
		}
	} else {
		records, err := providers.Search(ctx, id, query, i.settings, i.client)
		if err != nil {
			return nil, err
		}
		for _, record := range records {
			works = append(works, recordWork(source, record))
		}
	}
	a.mu.Lock()
	if len(a.cache) >= 256 {
		clear(a.cache)
	}
	a.cache[key] = cachedSearch{works: works, at: time.Now()}
	a.mu.Unlock()
	return works, nil
}

func releaseWork(source Source, release *cardigann.Release, requested string) Work {
	variant := Variant{ID: identity(source.ID, release.GUID, release.Title, release.Link, release.Magnet), SourceID: source.ID, SourceName: source.Name, Title: release.Title, URL: release.Details, Transport: "torrent", Magnet: release.Magnet, DownloadURL: release.Link, InfoHash: strings.ToLower(release.InfoHash), PublishedAt: release.PublishDate}
	if variant.URL == "" {
		variant.URL = source.URL
	}
	if release.Size > 0 {
		size := release.Size
		variant.Size = &size
	}
	if release.Seeders >= 0 && !syntheticField(source.definition, "seeders") {
		seeders := release.Seeders
		variant.Seeders = &seeders
	}
	if syntheticField(source.definition, "date") {
		variant.PublishedAt = ""
	}
	if variant.Magnet != "" && !validMagnet(variant.Magnet) {
		variant.Magnet = ""
	}
	id := variant.InfoHash
	if id == "" {
		id = identity(source.ID, release.GUID, release.Details, variant.ID)
	}
	return Work{ID: "torrent-" + id, Title: release.Title, Category: releaseCategory(release.Categories, requested), URL: variant.URL, Cover: release.Poster, Description: release.Description, Year: release.Year, Variants: []Variant{variant}}
}

func syntheticField(def *loader.Definition, name string) bool {
	if def == nil {
		return false
	}
	for _, field := range def.Search.Fields.Ordered() {
		if field.Key == name && field.Value.Text != nil {
			value := field.Value.Text.String()
			return !strings.Contains(value, "{{")
		}
	}
	return false
}

func recordWork(source Source, record providers.Record) Work {
	category := "all"
	if len(source.Categories) == 1 {
		category = source.Categories[0]
	}
	variant := Variant{ID: identity(source.ID, record.URL, record.DownloadURL, record.Magnet, record.Title), SourceID: source.ID, SourceName: source.Name, Title: record.Title, URL: record.URL, Transport: "torrent", Magnet: record.Magnet, DownloadURL: record.DownloadURL, InfoHash: record.InfoHash, PublishedAt: record.PublishedAt}
	if record.Size > 0 {
		size := record.Size
		variant.Size = &size
	}
	if record.Seeders >= 0 {
		seeders := record.Seeders
		variant.Seeders = &seeders
	}
	work := Work{ID: "torrent-" + identity(source.ID, record.URL), Title: record.Title, Category: category, URL: record.URL, Cover: record.Cover, Description: record.Description, Variants: []Variant{variant}}
	if len(record.Variants) > 0 {
		work.Variants = nil
		for _, child := range record.Variants {
			work.Variants = append(work.Variants, recordWork(source, child).Variants...)
		}
	}
	return work
}

func validMagnet(raw string) bool {
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "magnet" || len(raw) > 64<<10 {
		return false
	}
	for _, xt := range u.Query()["xt"] {
		if strings.HasPrefix(xt, "urn:btih:") {
			hash := strings.TrimPrefix(xt, "urn:btih:")
			if len(hash) == 40 {
				_, err := hex.DecodeString(hash)
				if err == nil {
					return true
				}
			}
			if len(hash) == 32 {
				valid := true
				for _, c := range strings.ToUpper(hash) {
					if !(c >= 'A' && c <= 'Z' || c >= '2' && c <= '7') {
						valid = false
					}
				}
				if valid {
					return true
				}
			}
		}
		if strings.HasPrefix(xt, "urn:btmh:1220") && len(xt) == 77 {
			_, err := hex.DecodeString(strings.TrimPrefix(xt, "urn:btmh:1220"))
			if err == nil {
				return true
			}
		}
	}
	return false
}
