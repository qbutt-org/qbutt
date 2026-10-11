package providers

import (
	"errors"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/PuerkitoBio/goquery"
	"golang.org/x/text/encoding/charmap"
)

func (s *session) searchRuTracker(query string) ([]Record, error) {
	encoded, err := charmap.Windows1251.NewEncoder().String(query)
	if err != nil {
		return nil, errors.New("RuTracker search requires Windows-1251 characters")
	}
	address := s.source.URL + "/forum/tracker.php?" + url.Values{"nm": {encoded}, "o": {"10"}, "s": {"2"}}.Encode()
	doc, err := s.document(address, s.source.URL+"/forum/")
	if err != nil {
		return nil, err
	}
	if doc.Find("#logged-in-username").Length() == 0 {
		if err := s.loginRuTracker(); err != nil {
			return nil, err
		}
		doc, err = s.document(address, s.source.URL+"/forum/")
		if err != nil {
			return nil, err
		}
	}
	if doc.Find("#logged-in-username").Length() == 0 {
		return nil, ErrLoginRequired
	}
	var records []Record
	doc.Find("#tor-tbl > tbody > tr").Each(func(_ int, row *goquery.Selection) {
		link := row.Find("td.t-title-col a.tLink[data-topic_id]").First()
		id, _ := link.Attr("data-topic_id")
		if _, err := strconv.ParseUint(id, 10, 64); err != nil {
			return
		}
		download, ok := row.Find("td.tor-size > a.tr-dl").Attr("href")
		if !ok {
			return
		}
		address := s.source.URL + "/forum/viewtopic.php?t=" + id
		record := Record{
			Title:       strings.Join(strings.Fields(link.Text()), " "),
			URL:         address,
			DownloadURL: absoluteURL(s.source.URL+"/forum/", download),
			Size:        -1,
			Seeders:     -1,
		}
		if text, ok := row.Find("td.tor-size").Attr("data-ts_text"); ok {
			if size, err := strconv.ParseInt(text, 10, 64); err == nil && size >= 0 {
				record.Size = size
			}
		}
		text := strings.TrimSpace(row.Find("td:nth-child(7) b").Text())
		if seeders, err := strconv.ParseInt(text, 10, 64); err == nil && seeders >= 0 {
			record.Seeders = seeders
		}
		if timestamp, ok := row.Find("td:nth-child(10)").Attr("data-ts_text"); ok {
			if seconds, err := strconv.ParseInt(timestamp, 10, 64); err == nil && seconds > 0 {
				record.PublishedAt = time.Unix(seconds, 0).UTC().Format(time.RFC3339)
			}
		}
		if record.Title != "" {
			records = append(records, record)
		}
	})
	if len(records) == 0 && doc.Find("#tor-tbl, #search-results").Length() == 0 && !strings.Contains(strings.ToLower(doc.Text()), "не найден") {
		return nil, errors.New("RuTracker did not return a search results page")
	}
	return records, nil
}

func ruTrackerNeedsLogin(doc *goquery.Document) bool {
	return doc.Find("input[name='login_username']").Length() > 0 && doc.Find("#logged-in-username").Length() == 0
}

func (s *session) loginRuTracker() error {
	if s.settings["username"] == "" || s.settings["password"] == "" {
		return ErrLoginRequired
	}
	values := url.Values{}
	values.Set("redirect", "index.php")
	for name, value := range map[string]string{
		"login_username": s.settings["username"],
		"login_password": s.settings["password"],
		"login":          "Вход",
	} {
		encoded, err := charmap.Windows1251.NewEncoder().String(value)
		if err != nil {
			return errors.New("RuTracker credentials require Windows-1251 characters")
		}
		values.Set(name, encoded)
	}
	address := s.source.URL + "/forum/login.php"
	data, contentType, err := s.request(http.MethodPost, address, address, values.Encode())
	if err != nil {
		return err
	}
	doc, err := parseDocument(data, contentType)
	if err != nil {
		return err
	}
	if doc.Find("input[name='cap_sid'], input[name='g-recaptcha-response'], .g-recaptcha").Length() > 0 {
		return ErrChallenge
	}
	if doc.Find("#logged-in-username").Length() == 0 {
		return ErrLoginRequired
	}
	return nil
}

func (s *session) inspectRuTracker(address *url.URL) (Record, error) {
	id := address.Query().Get("t")
	if _, err := strconv.ParseUint(id, 10, 64); err != nil {
		return Record{}, errors.New("invalid RuTracker topic")
	}
	page := s.source.URL + "/forum/viewtopic.php?t=" + id
	doc, err := s.document(page, s.source.URL+"/forum/")
	if err != nil {
		return Record{}, err
	}
	if ruTrackerNeedsLogin(doc) {
		if err := s.loginRuTracker(); err != nil {
			return Record{}, err
		}
		doc, err = s.document(page, s.source.URL+"/forum/")
		if err != nil {
			return Record{}, err
		}
	}
	body := doc.Find(".post_body").First()
	if body.Length() == 0 {
		if ruTrackerNeedsLogin(doc) {
			return Record{}, ErrLoginRequired
		}
		return Record{}, errors.New("RuTracker topic was not found")
	}
	record := Record{
		Title:       strings.TrimSpace(doc.Find("#topic-title").Text()),
		URL:         page,
		DownloadURL: s.source.URL + "/forum/dl.php?t=" + id,
		Description: plainText(body),
		Cover:       imageURL(body, page),
		Size:        -1,
		Seeders:     -1,
	}
	if magnet, ok := doc.Find("table.attach a.magnet-link[href^='magnet:']").First().Attr("href"); ok {
		record.Magnet = magnet
		record.InfoHash = magnetHash(magnet)
	}
	return record, nil
}
