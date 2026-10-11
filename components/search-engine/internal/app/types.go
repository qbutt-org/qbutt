package app

import "qbutt.org/search-engine/internal/indexer/cardigann/loader"

type Request struct {
	ID     int    `json:"id"`
	Method string `json:"method"`
	Params Params `json:"params"`
}

type Params struct {
	Query      string            `json:"query"`
	Category   string            `json:"category"`
	Expanded   bool              `json:"expanded"`
	Sources    []string          `json:"sources"`
	Variant    Variant           `json:"variant"`
	ID         string            `json:"id"`
	Enabled    *bool             `json:"enabled"`
	Settings   map[string]string `json:"settings"`
	Cookies    map[string]string `json:"cookies"`
	UserAgents map[string]string `json:"userAgents"`
	RequestID  int               `json:"requestId"`
	URL        string            `json:"url"`
}

type Event struct {
	ID    int    `json:"id"`
	Event string `json:"event"`
	Data  any    `json:"data,omitempty"`
}

type Source struct {
	ID            string    `json:"id"`
	Name          string    `json:"name"`
	URL           string    `json:"url"`
	Categories    []string  `json:"categories"`
	Enabled       bool      `json:"enabled"`
	RequiresLogin bool      `json:"requiresLogin,omitempty"`
	Settings      []Setting `json:"settings,omitempty"`
	definition    *loader.Definition
}

type Setting struct {
	Name       string   `json:"name"`
	Label      string   `json:"label"`
	Type       string   `json:"type"`
	Value      any      `json:"value,omitempty"`
	Options    []Option `json:"options,omitempty"`
	Configured bool     `json:"configured,omitempty"`
	Advanced   bool     `json:"advanced,omitempty"`
}

type Option struct {
	Value string `json:"value"`
	Label string `json:"label"`
}

type Variant struct {
	ID          string   `json:"id"`
	SourceID    string   `json:"sourceId"`
	SourceName  string   `json:"sourceName"`
	Title       string   `json:"title"`
	URL         string   `json:"url"`
	Transport   string   `json:"transport"`
	Magnet      string   `json:"magnet,omitempty"`
	DownloadURL string   `json:"downloadUrl,omitempty"`
	InfoHash    string   `json:"infoHash,omitempty"`
	Size        *int64   `json:"size,omitempty"`
	Seeders     *int64   `json:"seeders,omitempty"`
	PublishedAt string   `json:"publishedAt,omitempty"`
	Version     string   `json:"version,omitempty"`
	Edition     string   `json:"edition,omitempty"`
	Languages   []string `json:"languages,omitempty"`
	Quality     string   `json:"quality,omitempty"`
}

type Work struct {
	ID          string    `json:"id"`
	Title       string    `json:"title"`
	Category    string    `json:"category"`
	URL         string    `json:"url,omitempty"`
	Cover       string    `json:"cover,omitempty"`
	Description string    `json:"description,omitempty"`
	Year        int64     `json:"year,omitempty"`
	Variants    []Variant `json:"variants"`
}

type Resolved struct {
	Kind        string `json:"kind"`
	URL         string `json:"url,omitempty"`
	TorrentData string `json:"torrentData,omitempty"`
	Title       string `json:"title,omitempty"`
}
