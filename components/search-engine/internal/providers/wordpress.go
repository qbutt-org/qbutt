package providers

import (
	"errors"
	"net/url"
	"strings"

	"github.com/PuerkitoBio/goquery"
)

func (s *session) searchWordPress(query string) ([]Record, error) {
	address := s.source.URL + "/?" + url.Values{"s": {query}}.Encode()
	doc, err := s.document(address, s.source.URL)
	if err != nil {
		return nil, err
	}
	var records []Record
	doc.Find("article").Each(func(_ int, article *goquery.Selection) {
		link := article.Find(".entry-title a").First()
		href, _ := link.Attr("href")
		href = absoluteURL(address, href)
		if href == "" || !sameHost(href, s.source.URL) {
			return
		}
		record := Record{
			Title:       strings.Join(strings.Fields(link.Text()), " "),
			URL:         href,
			Description: plainText(article.Find(".entry-summary")),
			Cover:       imageURL(article, address),
			Size:        -1,
			Seeders:     -1,
		}
		record.PublishedAt, _ = article.Find("time[datetime]").First().Attr("datetime")
		if record.Title != "" {
			records = append(records, record)
		}
	})
	if len(records) == 0 && doc.Find(".no-results, .search-no-results, .nothing-found").Length() == 0 && doc.Find("article").Length() == 0 {
		return nil, errors.New("source did not return a WordPress search page")
	}
	return records, nil
}

func (s *session) inspectWordPress(doc *goquery.Document, address string) (Record, error) {
	article := doc.Find("article").First()
	body := article.Find(".entry-content").First()
	if body.Length() == 0 {
		return Record{}, errors.New("release page was not found")
	}
	record := Record{
		Title:       strings.TrimSpace(article.Find(".entry-title").First().Text()),
		URL:         address,
		Description: plainText(body),
		Cover:       imageURL(body, address),
		Size:        -1,
		Seeders:     -1,
	}
	record.PublishedAt, _ = article.Find("time[datetime]").First().Attr("datetime")
	if cover, ok := doc.Find("meta[property='og:image']").Attr("content"); ok {
		record.Cover = absoluteURL(address, cover)
	}
	attachVariants(&record, torrentLinks(body, record))
	return record, nil
}

func torrentLinks(body *goquery.Selection, record Record) []Record {
	var variants []Record
	body.Find("a[href]").Each(func(_ int, link *goquery.Selection) {
		href, _ := link.Attr("href")
		href = absoluteURL(record.URL, href)
		variant := Record{Title: record.Title, URL: record.URL, Size: -1, Seeders: -1}
		if strings.HasPrefix(href, "magnet:") {
			variant.InfoHash = magnetHash(href)
			if variant.InfoHash == "" {
				return
			}
			variant.Magnet = href
		} else if isTorrentURL(href) {
			variant.DownloadURL = href
			if title := strings.TrimSpace(link.Text()); title != "" && title != "magnet" {
				variant.Title = title
			}
		} else {
			return
		}
		variants = append(variants, variant)
	})
	return variants
}

func isTorrentURL(address string) bool {
	u, err := url.Parse(address)
	return err == nil && strings.HasSuffix(strings.ToLower(u.Path), ".torrent")
}
