package app

import (
	"context"
	"errors"
	"strings"
	"time"

	"github.com/PuerkitoBio/goquery"
	"golang.org/x/net/html/charset"

	"qbutt.org/search-engine/internal/providers"
)

func (a *App) remember(work Work) {
	a.mu.Lock()
	defer a.mu.Unlock()
	for _, variant := range work.Variants {
		a.variants[variant.ID] = storedRelease{Variant: variant, SeenAt: time.Now()}
	}
}

func (a *App) knownVariant(input Variant) (Variant, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	stored, ok := a.variants[input.ID]
	if !ok || stored.Variant.SourceID != input.SourceID {
		return Variant{}, errors.New("search for this release again")
	}
	if input.InfoHash != "" && !strings.EqualFold(stored.Variant.InfoHash, input.InfoHash) {
		return Variant{}, errors.New("the saved release identity differs")
	}
	return stored.Variant, nil
}

func (a *App) inspect(ctx context.Context, input Variant) (Work, error) {
	variant, err := a.knownVariant(input)
	if err != nil {
		return a.rehydrate(ctx, input)
	}
	source, i, err := a.sourceInstance(variant.SourceID)
	if err != nil {
		return Work{}, err
	}
	work, err := a.inspectPrepared(ctx, variant, source, i)
	if err != nil {
		return Work{}, err
	}
	a.remember(work)
	if err := a.saveReleases(); err != nil {
		return Work{}, err
	}
	return work, nil
}

func (a *App) inspectPrepared(ctx context.Context, variant Variant, source Source, i *instance) (Work, error) {
	work := Work{ID: "torrent-" + variant.ID, Title: variant.Title, Category: "all", URL: variant.URL, Variants: []Variant{variant}}
	if i.engine == nil && !strings.HasPrefix(source.ID, "custom-") {
		record, err := providers.Inspect(ctx, source.ID, variant.URL, i.settings, i.client)
		if err != nil {
			return Work{}, err
		}
		work = recordWork(source, record)
	} else {
		body, contentType, err := fetchPage(ctx, i.client, variant.URL)
		if err != nil {
			return Work{}, err
		}
		reader, err := charset.NewReader(strings.NewReader(string(body)), contentType)
		if err != nil {
			return Work{}, err
		}
		doc, err := goquery.NewDocumentFromReader(reader)
		if err != nil {
			return Work{}, err
		}
		doc.Find("script,style,nav,header,footer,iframe").Remove()
		main := doc.Find("article,main,#details,#t-details,.post_body,.post_wrap").First()
		if main.Length() == 0 {
			main = doc.Find("body")
		}
		text := strings.Join(strings.Fields(main.Text()), " ")
		if len(text) > 12000 {
			text = text[:12000]
		}
		work.Description = text
		if cover, ok := doc.Find("meta[property='og:image']").Attr("content"); ok {
			work.Cover = absolute(variant.URL, cover)
		}
		if variant.DownloadURL == "" && variant.Magnet == "" {
			work.Variants = nil
			if title := strings.TrimSpace(main.Find("h1,h2.title,.post_title").First().Text()); title != "" {
				work.Title = title
			}
		}
		seen := map[string]bool{variant.Magnet: true, variant.DownloadURL: true}
		downloads := doc.Find("#download,#download-block,#download_links,.download,.downloads,table.attach,.attach")
		if downloads.Length() == 0 {
			downloads = main
		}
		downloads.Find("a[href]").Each(func(_ int, link *goquery.Selection) {
			if len(work.Variants) >= 100 {
				return
			}
			href, _ := link.Attr("href")
			href = absolute(variant.URL, href)
			if seen[href] || (!validMagnet(href) && !strings.HasSuffix(strings.Split(href, "?")[0], ".torrent")) {
				return
			}
			seen[href] = true
			choice := variant
			choice.ID = identity(source.ID, href)
			choice.Magnet = ""
			choice.DownloadURL = ""
			choice.InfoHash = ""
			if title := strings.TrimSpace(link.Text()); len(title) > 12 {
				choice.Title = title
			}
			if validMagnet(href) {
				choice.Magnet = href
				choice.InfoHash = magnetHash(href)
			} else {
				choice.DownloadURL = href
			}
			work.Variants = append(work.Variants, choice)
		})
	}
	return work, nil
}

func (a *App) resolve(ctx context.Context, input Variant) (Resolved, error) {
	variant, err := a.knownVariant(input)
	if err != nil {
		work, err := a.rehydrate(ctx, input)
		if err != nil {
			return Resolved{}, err
		}
		if len(work.Variants) != 1 {
			return Resolved{}, providers.ErrChooseVariant
		}
		variant = work.Variants[0]
	}
	if validMagnet(variant.Magnet) {
		return torrentResolved(variant, nil, variant.Magnet)
	}
	source, i, err := a.sourceInstance(variant.SourceID)
	if err != nil {
		return Resolved{}, err
	}
	link := variant.DownloadURL
	if link == "" {
		link = variant.URL
	}
	if i.engine != nil {
		result, err := i.engine.Grab(ctx, link)
		if err != nil {
			return Resolved{}, err
		}
		return torrentResolved(variant, result.Body, result.Redirect)
	}
	if strings.HasPrefix(source.ID, "custom-") {
		body, _, err := fetchPage(ctx, i.client, link)
		if err != nil {
			return Resolved{}, err
		}
		return torrentResolved(variant, body, "")
	}
	body, magnet, err := providers.Resolve(ctx, source.ID, link, i.settings, i.client)
	if err != nil {
		return Resolved{}, err
	}
	return torrentResolved(variant, body, magnet)
}

func (a *App) rehydrate(ctx context.Context, input Variant) (Work, error) {
	source, i, err := a.sourceInstance(input.SourceID)
	if err != nil {
		return Work{}, err
	}
	if !a.ownsURL(source, input.URL) {
		return Work{}, errors.New("release address does not belong to the selected source")
	}
	variant := Variant{ID: identity(source.ID, input.URL), SourceID: source.ID, SourceName: source.Name, Title: input.Title, URL: input.URL, Transport: "torrent"}
	work, err := a.inspectPrepared(ctx, variant, source, i)
	if err != nil {
		return Work{}, err
	}
	expected := strings.ToLower(input.InfoHash)
	if expected == "" {
		expected = magnetHash(input.Magnet)
	}
	if expected != "" {
		var matched []Variant
		for _, choice := range work.Variants {
			actual := strings.ToLower(choice.InfoHash)
			if actual == "" {
				actual = magnetHash(choice.Magnet)
			}
			if actual == "" && choice.DownloadURL != "" && (choice.ID == input.ID || len(work.Variants) <= 3) {
				var data []byte
				if i.engine != nil {
					grab, err := i.engine.Grab(ctx, choice.DownloadURL)
					if err == nil {
						data = grab.Body
						actual = magnetHash(grab.Redirect)
					}
				} else if strings.HasPrefix(source.ID, "custom-") {
					data, _, _ = fetchPage(ctx, i.client, choice.DownloadURL)
				} else {
					data, _, _ = providers.Resolve(ctx, source.ID, choice.DownloadURL, i.settings, i.client)
				}
				if actual == "" {
					actual = torrentHash(data, len(expected) == 64)
				}
			}
			if actual == expected {
				choice.InfoHash = actual
				matched = append(matched, choice)
			}
		}
		if len(matched) == 0 {
			return Work{}, errors.New("this exact release could not be verified at its source")
		}
		work.Variants = matched[:1]
	} else if input.Version != "" && !strings.Contains(strings.ToLower(work.Title), strings.ToLower(input.Version)) {
		return Work{}, errors.New("the saved version is no longer offered by this source")
	}
	if len(work.Variants) == 0 {
		return Work{}, providers.ErrNoDownload
	}
	a.remember(work)
	if err := a.saveReleases(); err != nil {
		return Work{}, err
	}
	return work, nil
}
