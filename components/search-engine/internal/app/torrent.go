package app

import (
	"crypto/sha1"
	"crypto/sha256"
	"encoding/base32"
	"encoding/hex"
	"net/url"
	"strings"
)

type bencodeReader struct {
	data     []byte
	position int
	items    int
	info     []byte
}

func (r *bencodeReader) bytes() ([]byte, bool) {
	length, digits := 0, 0
	for r.position < len(r.data) && r.data[r.position] >= '0' && r.data[r.position] <= '9' {
		length = length*10 + int(r.data[r.position]-'0')
		r.position++
		digits++
		if digits > 8 {
			return nil, false
		}
	}
	if digits == 0 || r.position >= len(r.data) || r.data[r.position] != ':' {
		return nil, false
	}
	r.position++
	if length > len(r.data)-r.position {
		return nil, false
	}
	data := r.data[r.position : r.position+length]
	r.position += length
	return data, true
}

func (r *bencodeReader) value(depth int) bool {
	r.items++
	if depth > 64 || r.items > 200000 || r.position >= len(r.data) {
		return false
	}
	switch r.data[r.position] {
	case 'd':
		r.position++
		for r.position < len(r.data) && r.data[r.position] != 'e' {
			key, ok := r.bytes()
			if !ok {
				return false
			}
			start := r.position
			if !r.value(depth + 1) {
				return false
			}
			if depth == 0 && string(key) == "info" && r.data[start] == 'd' {
				r.info = r.data[start:r.position]
			}
		}
		if r.position >= len(r.data) {
			return false
		}
		r.position++
		return true
	case 'l':
		r.position++
		for r.position < len(r.data) && r.data[r.position] != 'e' {
			if !r.value(depth + 1) {
				return false
			}
		}
		if r.position >= len(r.data) {
			return false
		}
		r.position++
		return true
	case 'i':
		r.position++
		if r.position < len(r.data) && r.data[r.position] == '-' {
			r.position++
		}
		start := r.position
		for r.position < len(r.data) && r.data[r.position] >= '0' && r.data[r.position] <= '9' {
			r.position++
		}
		if r.position == start || r.position >= len(r.data) || r.data[r.position] != 'e' {
			return false
		}
		r.position++
		return true
	default:
		_, ok := r.bytes()
		return ok
	}
}

func torrentInfo(data []byte) []byte {
	if len(data) == 0 || len(data) > 8<<20 || data[0] != 'd' {
		return nil
	}
	r := bencodeReader{data: data}
	if !r.value(0) || r.position != len(data) {
		return nil
	}
	return r.info
}

func validTorrent(data []byte) bool { return len(torrentInfo(data)) > 0 }

func torrentHash(data []byte, v2 bool) string {
	info := torrentInfo(data)
	if len(info) == 0 {
		return ""
	}
	if v2 {
		digest := sha256.Sum256(info)
		return hex.EncodeToString(digest[:])
	}
	digest := sha1.Sum(info)
	return hex.EncodeToString(digest[:])
}

func magnetHash(raw string) string {
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "magnet" {
		return ""
	}
	for _, xt := range u.Query()["xt"] {
		if strings.HasPrefix(xt, "urn:btih:") {
			hash := strings.TrimPrefix(xt, "urn:btih:")
			if len(hash) == 40 {
				if _, err := hex.DecodeString(hash); err == nil {
					return strings.ToLower(hash)
				}
			}
			if len(hash) == 32 {
				if decoded, err := base32.StdEncoding.DecodeString(strings.ToUpper(hash)); err == nil {
					return hex.EncodeToString(decoded)
				}
			}
		}
		if strings.HasPrefix(xt, "urn:btmh:1220") && len(xt) == 77 {
			hash := strings.TrimPrefix(xt, "urn:btmh:1220")
			if _, err := hex.DecodeString(hash); err == nil {
				return strings.ToLower(hash)
			}
		}
	}
	return ""
}
