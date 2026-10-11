package app

import (
	"encoding/json"
	"errors"
	"maps"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	apphttp "qbutt.org/search-engine/internal/http"
)

type storedRelease struct {
	Variant Variant   `json:"variant"`
	SeenAt  time.Time `json:"seenAt"`
}

func loadReleases(dir string) (map[string]storedRelease, error) {
	list := map[string]storedRelease{}
	data, err := os.ReadFile(filepath.Join(dir, "releases.dat"))
	if errors.Is(err, os.ErrNotExist) {
		return list, nil
	}
	if err != nil {
		return nil, err
	}
	plain, err := unprotect(data)
	if err != nil {
		return nil, errors.New("saved releases could not be unlocked for this Windows account")
	}
	if err := json.Unmarshal(plain, &list); err != nil {
		return nil, errors.New("saved releases are invalid")
	}
	return list, nil
}

func (a *App) saveReleases() error {
	a.releaseMu.Lock()
	defer a.releaseMu.Unlock()
	a.mu.Lock()
	if len(a.variants) > 20000 {
		ids := slices.Sorted(maps.Keys(a.variants))
		slices.SortFunc(ids, func(left, right string) int { return a.variants[left].SeenAt.Compare(a.variants[right].SeenAt) })
		for _, id := range ids[:len(ids)-20000] {
			delete(a.variants, id)
		}
	}
	data, err := json.Marshal(a.variants)
	a.mu.Unlock()
	if err != nil {
		return err
	}
	protected, err := protect(data)
	if err != nil {
		return err
	}
	return atomicWrite(filepath.Join(a.dir, "releases.dat"), protected)
}

func (a *App) publicWork(work Work) Work {
	if len(work.Title) > 8192 {
		work.Title = work.Title[:8192]
	}
	if len(work.Description) > 12000 {
		work.Description = work.Description[:12000]
	}
	work.URL = apphttp.RedactURLIdentity(work.URL)
	work.Cover = apphttp.RedactURLIdentity(work.Cover)
	work.Description = apphttp.RedactSecretsInText(work.Description)
	work.Variants = slices.Clone(work.Variants)
	a.mu.Lock()
	defer a.mu.Unlock()
	for index, variant := range work.Variants {
		source := a.sources[variant.SourceID]
		variant.URL = apphttp.RedactURLIdentity(variant.URL)
		if apphttp.RedactURLIdentity(variant.DownloadURL) != variant.DownloadURL {
			variant.DownloadURL = ""
		}
		if apphttp.RedactURLIdentity(variant.Magnet) != variant.Magnet {
			variant.Magnet = ""
		} else if magnet, err := url.Parse(variant.Magnet); err == nil {
			for _, tracker := range magnet.Query()["tr"] {
				if apphttp.RedactURL(tracker) != tracker {
					variant.Magnet = ""
					break
				}
			}
		}
		if source.RequiresLogin {
			variant.DownloadURL = ""
			variant.Magnet = ""
		}
		credentials := maps.Clone(a.preferences.Sources[source.ID].Settings)
		if credentials == nil {
			credentials = map[string]string{}
		}
		credentials["browserCookie"] = a.sessions[source.ID].Cookie
		for key, secret := range credentials {
			name := strings.ToLower(key)
			if secret == "" || (!strings.Contains(name, "pass") && !strings.Contains(name, "key") && !strings.Contains(name, "cookie") && !strings.Contains(name, "token") && name != "proxyurl") {
				continue
			}
			values := []string{secret}
			if strings.Contains(name, "cookie") {
				for _, part := range strings.Split(secret, ";") {
					if _, value, ok := strings.Cut(part, "="); ok && strings.TrimSpace(value) != "" {
						values = append(values, strings.TrimSpace(value))
					}
				}
			}
			for _, value := range values {
				for _, raw := range []string{value, url.QueryEscape(value), url.PathEscape(value)} {
					work.Title = strings.ReplaceAll(work.Title, raw, "[redacted]")
					work.Description = strings.ReplaceAll(work.Description, raw, "[redacted]")
					variant.Title = strings.ReplaceAll(variant.Title, raw, "[redacted]")
					if strings.Contains(work.URL, raw) {
						work.URL = apphttp.RedactURLIdentity(source.URL)
					}
					if strings.Contains(work.Cover, raw) {
						work.Cover = ""
					}
					if strings.Contains(variant.URL, raw) {
						variant.URL = apphttp.RedactURLIdentity(source.URL)
					}
					if strings.Contains(variant.DownloadURL, raw) {
						variant.DownloadURL = ""
					}
					if strings.Contains(variant.Magnet, raw) {
						variant.Magnet = ""
					}
				}
			}
		}
		work.Variants[index] = variant
	}
	return work
}

func (a *App) ownsURL(source Source, raw string) bool {
	u, err := checkURL(raw)
	if err != nil {
		return false
	}
	allowed := []string{source.URL}
	if source.definition != nil {
		allowed = append(allowed, source.definition.Links...)
	}
	a.mu.Lock()
	base := a.preferences.Sources[source.ID].Settings["baseUrl"]
	a.mu.Unlock()
	if base != "" {
		allowed = append(allowed, base)
	}
	for _, candidate := range allowed {
		origin, err := url.Parse(candidate)
		if err == nil && strings.EqualFold(origin.Host, u.Host) && origin.Scheme == u.Scheme {
			return true
		}
	}
	return false
}
