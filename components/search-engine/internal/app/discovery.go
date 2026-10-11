package app

import (
	"context"
	"encoding/xml"
	"errors"
	"io"
	"net/http"
	"net/url"
	"strings"

	"github.com/PuerkitoBio/goquery"
)

type feed struct {
	Channel struct {
		Items []feedItem `xml:"item"`
	} `xml:"channel"`
}

type feedItem struct {
	Title       string `xml:"title"`
	Link        string `xml:"link"`
	Comments    string `xml:"comments"`
	Description string `xml:"description"`
	Date        string `xml:"pubDate"`
	Enclosure   struct {
		URL  string `xml:"url,attr"`
		Type string `xml:"type,attr"`
	} `xml:"enclosure"`
	Hash    string `xml:"infoHash"`
	Seeders *int64 `xml:"seeders"`
}

func fetchPage(ctx context.Context, client *http.Client, raw string) ([]byte, string, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, raw, nil)
	if err != nil {
		return nil, "", err
	}
	response, err := client.Do(req)
	if err != nil {
		return nil, "", err
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return nil, "", errors.New("site refused the request")
	}
	body, err := io.ReadAll(response.Body)
	return body, response.Header.Get("Content-Type"), err
}

func absolute(base, link string) string {
	u, err := url.Parse(base)
	if err != nil {
		return ""
	}
	rel, err := url.Parse(link)
	if err != nil {
		return ""
	}
	resolved := u.ResolveReference(rel)
	if resolved.Scheme != "https" && resolved.Scheme != "http" && resolved.Scheme != "magnet" {
		return ""
	}
	return resolved.String()
}

func (a *App) discover(ctx context.Context, r Request) error {
	u, err := checkURL(strings.TrimSpace(r.Params.URL))
	if err != nil {
		return err
	}
	if _, err := resolvePublic(ctx, u.Hostname()); err != nil {
		return err
	}
	for _, source := range a.catalog() {
		known, _ := url.Parse(source.URL)
		if known != nil && strings.EqualFold(known.Hostname(), u.Hostname()) {
			enabled := true
			if err := a.configure(Params{ID: source.ID, Enabled: &enabled}); err != nil {
				return err
			}
			a.emit(r.ID, "sources", a.catalog())
			return nil
		}
	}
	client, err := newClient(nil, 0)
	if err != nil {
		return err
	}
	body, contentType, err := fetchPage(ctx, client, u.String())
	if err != nil {
		return err
	}
	site := customSite{Name: u.Hostname(), URL: u.String(), SearchURL: u.String(), Format: "html"}
	if strings.Contains(contentType, "xml") || strings.HasPrefix(strings.TrimSpace(string(body)), "<?xml") || strings.HasPrefix(strings.TrimSpace(string(body)), "<rss") {
		site.Format = "rss"
		if query := u.Query(); query.Has("q") {
			query.Set("q", "{query}")
			u.RawQuery = query.Encode()
			site.SearchURL = strings.ReplaceAll(u.String(), "%7Bquery%7D", "{query}")
		}
	} else {
		doc, err := goquery.NewDocumentFromReader(strings.NewReader(string(body)))
		if err != nil {
			return err
		}
		if title := strings.TrimSpace(doc.Find("title").First().Text()); title != "" {
			site.Name = title
		}
		found := false
		doc.Find("form").EachWithBreak(func(_ int, form *goquery.Selection) bool {
			method, _ := form.Attr("method")
			if method != "" && !strings.EqualFold(method, "get") {
				return true
			}
			action, _ := form.Attr("action")
			searchURL, _ := url.Parse(absolute(site.URL, action))
			if searchURL == nil {
				return true
			}
			params := searchURL.Query()
			form.Find("input[name]").Each(func(_ int, input *goquery.Selection) {
				name, _ := input.Attr("name")
				kind, _ := input.Attr("type")
				value, _ := input.Attr("value")
				if kind == "hidden" {
					params.Set(name, value)
				}
				if name == "q" || name == "s" || name == "search" || name == "query" || name == "story" {
					params.Set(name, "{query}")
					found = true
				}
			})
			if !found {
				return true
			}
			searchURL.RawQuery = params.Encode()
			site.SearchURL = strings.ReplaceAll(searchURL.String(), "%7Bquery%7D", "{query}")
			return false
		})
		if !found {
			doc.Find("a[href]").Each(func(_ int, link *goquery.Selection) {
				href, _ := link.Attr("href")
				href = absolute(site.URL, href)
				if validMagnet(href) || strings.HasSuffix(strings.Split(href, "?")[0], ".torrent") {
					found = true
				}
			})
			if !found {
				return errors.New("no supported search form or torrent feed was found")
			}
		}
	}
	query := strings.TrimSpace(r.Params.Query)
	id := "custom-" + identity(u.Hostname(), site.SearchURL)
	source := Source{ID: id, Name: site.Name, URL: site.URL, Categories: []string{"all"}, Enabled: true}
	works, err := customResults(ctx, source, site, client, query)
	if err != nil {
		return err
	}
	if len(works) == 0 {
		return errors.New("the site returned no torrent links; enter a representative query to verify it")
	}
	a.mu.Lock()
	enabled := true
	a.preferences.Custom[id] = site
	a.preferences.Sources[id] = sourcePreferences{Enabled: &enabled}
	if err := savePreferences(a.dir, a.preferences); err != nil {
		delete(a.preferences.Custom, id)
		delete(a.preferences.Sources, id)
		a.mu.Unlock()
		return err
	}
	a.sources[id] = source
	a.mu.Unlock()
	a.emit(r.ID, "sources", a.catalog())
	for _, work := range works {
		a.remember(work)
		a.emit(r.ID, "result", a.publicWork(work))
	}
	if err := a.saveReleases(); err != nil {
		return err
	}
	return nil
}

func (a *App) searchCustom(ctx context.Context, source Source, i *instance, query string) ([]Work, error) {
	a.mu.Lock()
	site := a.preferences.Custom[source.ID]
	a.mu.Unlock()
	return customResults(ctx, source, site, i.client, query)
}

func customResults(ctx context.Context, source Source, site customSite, client *http.Client, query string) ([]Work, error) {
	searchURL := strings.ReplaceAll(site.SearchURL, "{query}", url.QueryEscape(query))
	body, _, err := fetchPage(ctx, client, searchURL)
	if err != nil {
		return nil, err
	}
	if site.Format == "rss" {
		var document feed
		if err := xml.Unmarshal(body, &document); err != nil {
			return nil, err
		}
		var works []Work
		for _, item := range document.Channel.Items[:min(len(document.Channel.Items), 500)] {
			if !strings.Contains(site.SearchURL, "{query}") && query != "" && !strings.Contains(strings.ToLower(item.Title), strings.ToLower(query)) {
				continue
			}
			link := absolute(searchURL, item.Enclosure.URL)
			if !validMagnet(link) && !strings.Contains(strings.ToLower(item.Enclosure.Type), "bittorrent") && !strings.HasSuffix(strings.Split(link, "?")[0], ".torrent") {
				continue
			}
			variant := Variant{ID: identity(source.ID, item.Link, link), SourceID: source.ID, SourceName: source.Name, Title: item.Title, URL: absolute(searchURL, item.Link), Transport: "torrent", PublishedAt: item.Date, InfoHash: item.Hash, Seeders: item.Seeders}
			if validMagnet(link) {
				variant.Magnet = link
			} else {
				variant.DownloadURL = link
			}
			works = append(works, Work{ID: "torrent-" + variant.ID, Title: item.Title, Category: "all", URL: variant.URL, Description: item.Description, Variants: []Variant{variant}})
		}
		return works, nil
	}
	doc, err := goquery.NewDocumentFromReader(strings.NewReader(string(body)))
	if err != nil {
		return nil, err
	}
	var works []Work
	seen := map[string]bool{}
	doc.Find("a[href]").Each(func(_ int, link *goquery.Selection) {
		if len(works) >= 500 {
			return
		}
		href, _ := link.Attr("href")
		href = absolute(searchURL, href)
		if seen[href] || (!validMagnet(href) && !strings.HasSuffix(strings.Split(href, "?")[0], ".torrent")) {
			return
		}
		seen[href] = true
		title := strings.TrimSpace(link.Text())
		label := strings.Trim(strings.ToLower(title), "() ")
		if title == "" || len(title) < 8 || label == "torrent file" || label == "magnet link" || label == "download" {
			container := link.Closest("tr,article,li,.item")
			title = strings.TrimSpace(container.Find("h1,h2,h3,.title").First().Text())
			if title == "" {
				text := container.Clone()
				text.Find("a,script,style").Remove()
				title = strings.Join(strings.Fields(text.Text()), " ")
			}
		}
		if title == "" {
			title = query
		}
		if !strings.Contains(site.SearchURL, "{query}") && query != "" && !strings.Contains(strings.ToLower(title), strings.ToLower(query)) {
			return
		}
		variant := Variant{ID: identity(source.ID, href), SourceID: source.ID, SourceName: source.Name, Title: title, URL: searchURL, Transport: "torrent"}
		if validMagnet(href) {
			variant.Magnet = href
			variant.InfoHash = magnetHash(href)
		} else {
			variant.DownloadURL = href
		}
		works = append(works, Work{ID: "torrent-" + variant.ID, Title: title, Category: "all", URL: searchURL, Variants: []Variant{variant}})
	})
	return works, nil
}
