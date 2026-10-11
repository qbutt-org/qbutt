package providers

import (
	"bytes"
	"errors"
	"net/http"
	"net/url"
)

func (s *session) resolve(address string, depth int) ([]byte, string, error) {
	if depth > 2 {
		return nil, "", ErrNoDownload
	}
	u, err := url.Parse(address)
	if err != nil {
		return nil, "", err
	}
	isRuTrackerDownload := s.source.ID == "rutracker" && sameHost(address, s.source.URL) && u.Path == "/forum/dl.php"
	if isRuTrackerDownload || isTorrentURL(address) {
		referer := s.source.URL
		if isRuTrackerDownload {
			referer += "/forum/viewtopic.php?" + url.Values{"t": {u.Query().Get("t")}}.Encode()
		}
		for attempt := 0; ; attempt++ {
			data, contentType, err := s.request(http.MethodGet, address, referer, "")
			if err != nil {
				return nil, "", err
			}
			// The caller validates the complete bencoded metainfo before adding it.
			if bytes.HasPrefix(data, []byte("d")) && bytes.Contains(data, []byte("4:info")) {
				return data, "", nil
			}
			if !isRuTrackerDownload {
				return nil, "", errors.New("source did not return torrent metadata")
			}
			doc, err := parseDocument(data, contentType)
			if err != nil {
				return nil, "", err
			}
			if !ruTrackerNeedsLogin(doc) {
				return nil, "", errors.New("RuTracker did not return torrent metadata")
			}
			if attempt > 0 {
				return nil, "", ErrLoginRequired
			}
			if err := s.loginRuTracker(); err != nil {
				return nil, "", err
			}
		}
	}
	if !sameHost(address, s.source.URL) {
		return nil, "", ErrNoDownload
	}
	record, err := s.inspect(address)
	if err != nil {
		return nil, "", err
	}
	if len(record.Variants) > 1 {
		return nil, "", ErrChooseVariant
	}
	if record.Magnet != "" {
		return nil, record.Magnet, nil
	}
	if record.DownloadURL == "" || record.DownloadURL == address {
		return nil, "", ErrNoDownload
	}
	return s.resolve(record.DownloadURL, depth+1)
}
