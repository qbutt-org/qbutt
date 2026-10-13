package app

import (
	"errors"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"time"

	apphttp "qbutt.org/search-engine/internal/http"
	"qbutt.org/search-engine/internal/indexer/cardigann"
	"qbutt.org/search-engine/internal/indexer/cardigann/loader"
	"qbutt.org/search-engine/internal/indexer/cardigann/mapper"
	"qbutt.org/search-engine/internal/providers"
)

var defaultSources = map[string]bool{
	"rutor": true, "nyaasi": true, "nekobt": true, "noname-club": true,
	"yts": true, "eztv": true, "1337x": true, "limetorrents": true,
	"gamestorrents": true, "animelayer": true, "animebybelka": true,
}

// These are definition setting names, not translated display labels. Account
// credentials and site/API addresses remain primary; unknown new fields stay visible.
var advancedSourceSettings = map[string]bool{
	"sort": true, "type": true, "sort2": true, "sortby": true, "orderby": true,
	"order": true, "order_by": true, "order_dir": true, "disablesort": true,
	"stripcyrillic": true, "addrussiantotitle": true, "addrussian": true,
	"addukrainiantotitle": true, "add_version_francophone_to_title": true,
	"add_hybrid_features_to_filename": true, "incltags": true,
	"single_file_release_use_filename": true, "use_single_file_release_use_filename": true,
	"stripS01E01": true, "strip_s01": true, "dropbbc": true, "drop_polish_prefix": true,
	"titleOnly": true, "tmdbidonly": true, "top100": true, "page_for_test": true,
	"primarydownloadlink": true, "fallbackdownloadlink": true, "prefer_magnet_links": true, "noMagnet": true,
	"freeleech": true, "freeleech_50": true, "use_fl_tokens": true,
	"multilanguage": true, "multilang": true, "vostfr": true, "vfq": true,
	"audio_lang": true, "fansub_lang": true, "sub_lang": true, "withsubs": true, "hardsub": true, "lang": true,
	"uploader": true, "onlyupload": true, "p2p_releases_only": true, "unrar_releases_only": true,
	"ip_filtering": true, "cat-id": true, "filter-id": true, "category-id": true, "cat": true, "group_id": true,
	"filter-verified": true, "exclude_scene": true, "exclude_archives": true, "ignorevip": true,
	"dead": true, "hidef": true, "levels": true, "useragent": true,
}

func (a *App) loadCatalog() error {
	defs, _, err := loader.New(filepath.Join(a.dir, "definitions", "current")).LoadAll()
	if err != nil {
		return err
	}
	// Local definitions are independent of the replaceable upstream snapshot.
	byID := map[string]*loader.Definition{}
	for _, def := range defs {
		byID[def.ID] = def
	}
	localDir := filepath.Join(a.dir, "definitions", "local")
	localFiles, _ := filepath.Glob(filepath.Join(localDir, "*.yml"))
	for _, file := range localFiles {
		data, err := os.ReadFile(file)
		if err != nil {
			return err
		}
		def, err := loader.Parse(data)
		if err != nil {
			return errors.New("a local source definition is invalid")
		}
		byID[def.ID] = def
	}
	index := map[string]Source{}
	for _, def := range byID {
		caps, err := mapper.Build(def)
		if err != nil || len(def.Links) == 0 {
			continue
		}
		source := Source{ID: def.ID, Name: def.Name, URL: def.Links[0], Categories: categories(caps), RequiresLogin: def.Type != "public", definition: def}
		if def.ID == "rutor" {
			source.Categories = append([]string{"all"}, slices.Sorted(maps.Keys(rutorRoutes))...)
		}
		index[def.ID] = source
	}
	for _, native := range providers.Sources() {
		index[native.ID] = Source{ID: native.ID, Name: native.Name, URL: native.URL, Categories: native.Categories, RequiresLogin: native.RequiresLogin}
	}
	for id, site := range a.preferences.Custom {
		index[id] = Source{ID: id, Name: site.Name, URL: site.URL, Categories: []string{"all"}}
	}
	a.sources = index
	return nil
}

func matchesCategory(source Source, category string) bool {
	return category == "" || category == "all" || slices.Contains(source.Categories, "all") || slices.Contains(source.Categories, category)
}

func (a *App) catalog() []Source {
	a.mu.Lock()
	defer a.mu.Unlock()
	list := make([]Source, 0, len(a.sources))
	for _, source := range a.sources {
		p := a.preferences.Sources[source.ID]
		source.Enabled = defaultSources[source.ID]
		if source.ID == "rutracker" && a.sessions[source.ID].Cookie != "" {
			source.Enabled = true
		}
		if p.Enabled != nil {
			source.Enabled = *p.Enabled
		}
		if source.definition != nil {
			defaults := cardigann.DefaultConfig(source.definition)
			for _, field := range source.definition.Settings {
				if strings.HasPrefix(field.Type, "info") {
					continue
				}
				setting := Setting{Name: field.Name, Label: field.Label, Type: "text", Advanced: advancedSourceSettings[field.Name]}
				switch field.Type {
				case "checkbox":
					setting.Type = "boolean"
				case "select", "multi-select":
					setting.Type = "select"
				case "password":
					setting.Type = "password"
				}
				if field.IsSecret() {
					setting.Type = "password"
					setting.Configured = p.Settings[field.Name] != ""
				} else {
					value := defaults[field.Name]
					if saved, ok := p.Settings[field.Name]; ok {
						value = saved
					}
					if setting.Type == "boolean" {
						setting.Value = loader.CheckboxOn(value)
					} else {
						setting.Value = value
					}
				}
				for _, key := range slices.Sorted(maps.Keys(field.Options)) {
					setting.Options = append(setting.Options, Option{Value: key, Label: field.Options[key]})
				}
				source.Settings = append(source.Settings, setting)
			}
		} else if source.ID == "rutracker" {
			source.Settings = []Setting{{Name: "username", Label: "Username", Type: "text", Value: p.Settings["username"]}, {Name: "password", Label: "Password", Type: "password", Configured: p.Settings["password"] != ""}, {Name: "cookie", Label: "Browser session", Type: "password", Configured: p.Settings["cookie"] != ""}}
		}
		source.Settings = append(source.Settings, Setting{Name: "proxyUrl", Label: "Proxy address", Type: "password", Configured: p.Settings["proxyUrl"] != "", Advanced: true})
		source.URL = apphttp.RedactURLIdentity(source.URL)
		list = append(list, source)
	}
	sort.Slice(list, func(i, j int) bool {
		if list[i].Enabled != list[j].Enabled {
			return list[i].Enabled
		}
		if list[i].RequiresLogin != list[j].RequiresLogin {
			return !list[i].RequiresLogin
		}
		return strings.ToLower(list[i].Name) < strings.ToLower(list[j].Name)
	})
	return list
}

func (a *App) configure(p Params) error {
	a.mu.Lock()
	defer a.mu.Unlock()
	source, ok := a.sources[p.ID]
	if !ok {
		return errors.New("unknown source")
	}
	preferences := a.preferences.Sources[p.ID]
	preferences.Settings = maps.Clone(preferences.Settings)
	if preferences.Settings == nil {
		preferences.Settings = map[string]string{}
	}
	if p.Enabled != nil {
		preferences.Enabled = p.Enabled
	}
	allowed := map[string]bool{"baseUrl": true, "proxyUrl": true, "cookie": true, "useragent": true, "user_agent": true}
	if source.ID == "rutracker" {
		allowed["username"] = true
		allowed["password"] = true
	}
	if source.definition != nil {
		for _, setting := range source.definition.Settings {
			allowed[setting.Name] = true
		}
	}
	for key, value := range p.Settings {
		if !allowed[key] {
			return errors.New("unknown source setting")
		}
		if len(value) > 64<<10 {
			return errors.New("source setting is too long")
		}
		// The UI sends only changed fields; an explicit blank resets a setting.
		if value == "" {
			delete(preferences.Settings, key)
		} else {
			preferences.Settings[key] = value
		}
	}
	previous := a.preferences.Sources[p.ID]
	a.preferences.Sources[p.ID] = preferences
	if err := savePreferences(a.dir, a.preferences); err != nil {
		a.preferences.Sources[p.ID] = previous
		return err
	}
	delete(a.instances, p.ID)
	clear(a.cache)
	return nil
}

func sourceInterval(source Source) time.Duration {
	if source.definition != nil && source.definition.RequestDelay != nil {
		return time.Duration(*source.definition.RequestDelay * float64(time.Second))
	}
	return time.Second
}
