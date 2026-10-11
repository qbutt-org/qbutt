package app

import (
	"archive/tar"
	"compress/gzip"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"os"
	"path"
	"path/filepath"
	"strings"
	"time"

	"qbutt.org/search-engine/internal/indexer/cardigann"
	"qbutt.org/search-engine/internal/indexer/cardigann/loader"
	"qbutt.org/search-engine/internal/indexer/definitions"
)

type updateState struct {
	CheckedAt time.Time `json:"checkedAt"`
	ETag      string    `json:"etag"`
	Digest    string    `json:"digest"`
}

func recoverDefinitions(dir string) error {
	root := filepath.Join(dir, "definitions")
	current := filepath.Join(root, "current")
	previous := filepath.Join(root, "previous")
	if _, err := os.Stat(current); errors.Is(err, os.ErrNotExist) {
		if _, err := os.Stat(previous); err == nil {
			return os.Rename(previous, current)
		}
	}
	return nil
}

func readDefinition(dir, id string) ([]byte, error) {
	if strings.ContainsAny(id, "/\\.") {
		return nil, errors.New("invalid source identifier")
	}
	return os.ReadFile(filepath.Join(dir, id+".yml"))
}

func (a *App) update(ctx context.Context, force bool) error {
	a.updateMu.Lock()
	defer a.updateMu.Unlock()
	statePath := filepath.Join(a.dir, "definition-update.json")
	var state updateState
	if data, err := os.ReadFile(statePath); err == nil {
		_ = json.Unmarshal(data, &state)
	}
	if !force && time.Since(state.CheckedAt) < 24*time.Hour {
		return nil
	}
	ctx, cancel := context.WithTimeout(ctx, 90*time.Second)
	defer cancel()
	client := &http.Client{Timeout: 90 * time.Second}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, "https://codeload.github.com/Jackett/Jackett/tar.gz/refs/heads/master", nil)
	if err != nil {
		return err
	}
	if state.ETag != "" {
		req.Header.Set("If-None-Match", state.ETag)
	}
	req.Header.Set("User-Agent", "qbutt-search/1")
	response, err := client.Do(req)
	if err != nil {
		return errors.New("source update is unavailable; the saved catalogue is still ready")
	}
	defer response.Body.Close()
	if response.StatusCode == http.StatusNotModified {
		state.CheckedAt = time.Now()
		data, _ := json.Marshal(state)
		return atomicWrite(statePath, data)
	}
	if response.StatusCode != http.StatusOK {
		return errors.New("source update is unavailable; the saved catalogue is still ready")
	}
	root := filepath.Join(a.dir, "definitions")
	if err := os.MkdirAll(root, 0700); err != nil {
		return err
	}
	stage, err := os.MkdirTemp(root, "pending-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(stage)
	digest := sha256.New()
	zip, err := gzip.NewReader(io.TeeReader(io.LimitReader(response.Body, 128<<20), digest))
	if err != nil {
		return errors.New("source update archive is invalid")
	}
	defer zip.Close()
	reader := tar.NewReader(io.LimitReader(zip, 256<<20))
	count, total := 0, int64(0)
	for {
		header, err := reader.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return errors.New("source update archive is incomplete")
		}
		if header.Typeflag != tar.TypeReg {
			continue
		}
		prefix := "/src/Jackett.Common/Definitions/"
		position := strings.Index(header.Name, prefix)
		if position < 0 {
			continue
		}
		name := header.Name[position+len(prefix):]
		if path.Base(name) != name || !strings.HasSuffix(name, ".yml") {
			continue
		}
		if header.Size <= 0 || header.Size > 1<<20 {
			return errors.New("source update contains an oversized definition")
		}
		total += header.Size
		if total > 64<<20 || count >= 2000 {
			return errors.New("source update exceeds catalogue limits")
		}
		data, err := io.ReadAll(io.LimitReader(reader, header.Size))
		if err != nil {
			return err
		}
		def, parseErr := loader.Parse(data)
		if parseErr == nil {
			_, parseErr = cardigann.NewEngine(def)
		}
		if parseErr != nil {
			// A new schema operation cannot replace the last compatible source.
			old, readErr := readDefinition(filepath.Join(root, "current"), strings.TrimSuffix(name, ".yml"))
			if readErr != nil {
				old, readErr = definitions.Vendored.ReadFile("vendor/" + name)
			}
			if readErr != nil {
				continue
			}
			data = old
		}
		if err := os.WriteFile(filepath.Join(stage, name), data, 0600); err != nil {
			return err
		}
		count++
	}
	if count < 400 {
		return errors.New("source update was rejected because its catalogue is incomplete")
	}
	newDigest := hex.EncodeToString(digest.Sum(nil))
	if newDigest != state.Digest {
		previous := filepath.Join(root, "previous")
		current := filepath.Join(root, "current")
		if err := os.RemoveAll(previous); err != nil {
			return err
		}
		if _, err := os.Stat(current); err == nil {
			if err := os.Rename(current, previous); err != nil {
				return err
			}
		}
		if err := os.Rename(stage, current); err != nil {
			_ = os.Rename(previous, current)
			return err
		}
		a.mu.Lock()
		err := a.loadCatalog()
		clear(a.instances)
		clear(a.cache)
		a.mu.Unlock()
		if err != nil {
			_ = os.RemoveAll(current)
			_ = os.Rename(previous, current)
			a.mu.Lock()
			_ = a.loadCatalog()
			a.mu.Unlock()
			return errors.New("source update could not be activated; the previous catalogue was restored")
		}
	}
	state = updateState{CheckedAt: time.Now(), ETag: response.Header.Get("ETag"), Digest: newDigest}
	data, _ := json.Marshal(state)
	return atomicWrite(statePath, data)
}

func isParserError(err error) bool {
	s := strings.ToLower(err.Error())
	return strings.Contains(s, "template") || strings.Contains(s, "selector") || strings.Contains(s, "parse error")
}

func (a *App) previousSource(id string) ([]byte, *loader.Definition) {
	a.updateMu.Lock()
	defer a.updateMu.Unlock()
	root := filepath.Join(a.dir, "definitions")
	if _, err := os.Stat(filepath.Join(root, "local", id+".yml")); err == nil {
		return nil, nil
	}
	current, err := readDefinition(filepath.Join(root, "current"), id)
	if err != nil {
		return nil, nil
	}
	previous, err := readDefinition(filepath.Join(root, "previous"), id)
	if err != nil {
		previous, err = definitions.Vendored.ReadFile("vendor/" + id + ".yml")
	}
	if err != nil || string(current) == string(previous) {
		return nil, nil
	}
	def, err := loader.Parse(previous)
	if err != nil {
		return nil, nil
	}
	return previous, def
}

func (a *App) restoreSource(id string, expected, previous *loader.Definition, data []byte) {
	a.updateMu.Lock()
	defer a.updateMu.Unlock()
	a.mu.Lock()
	defer a.mu.Unlock()
	source := a.sources[id]
	if source.definition != expected {
		return
	}
	if err := atomicWrite(filepath.Join(a.dir, "definitions", "current", id+".yml"), data); err != nil {
		return
	}
	source.definition = previous
	a.sources[id] = source
	delete(a.instances, id)
}
