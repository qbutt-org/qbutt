package app

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
)

type preferences struct {
	Sources map[string]sourcePreferences `json:"sources"`
	Custom  map[string]customSite        `json:"custom,omitempty"`
}

type sourcePreferences struct {
	Enabled  *bool             `json:"enabled,omitempty"`
	Settings map[string]string `json:"settings,omitempty"`
}

type customSite struct {
	Name      string `json:"name"`
	URL       string `json:"url"`
	SearchURL string `json:"searchUrl"`
	Format    string `json:"format"`
}

func loadPreferences(dir string) (preferences, error) {
	p := preferences{Sources: map[string]sourcePreferences{}, Custom: map[string]customSite{}}
	data, err := os.ReadFile(filepath.Join(dir, "sources.dat"))
	if errors.Is(err, os.ErrNotExist) {
		return p, nil
	}
	if err != nil {
		return p, fmt.Errorf("read source preferences: %w", err)
	}
	plain, err := unprotect(data)
	if err != nil {
		return p, errors.New("saved source settings could not be unlocked for this Windows account")
	}
	if err := json.Unmarshal(plain, &p); err != nil {
		return p, errors.New("saved source settings are invalid")
	}
	if p.Sources == nil {
		p.Sources = map[string]sourcePreferences{}
	}
	if p.Custom == nil {
		p.Custom = map[string]customSite{}
	}
	return p, nil
}

func savePreferences(dir string, p preferences) error {
	plain, err := json.Marshal(p)
	if err != nil {
		return err
	}
	data, err := protect(plain)
	if err != nil {
		return errors.New("source settings could not be protected for this Windows account")
	}
	return atomicWrite(filepath.Join(dir, "sources.dat"), data)
}

func atomicWrite(path string, data []byte) error {
	file, err := os.CreateTemp(filepath.Dir(path), ".pending-*")
	if err != nil {
		return err
	}
	temp := file.Name()
	defer os.Remove(temp)
	if _, err := file.Write(data); err != nil {
		file.Close()
		return err
	}
	if err := file.Sync(); err != nil {
		file.Close()
		return err
	}
	if err := file.Close(); err != nil {
		return err
	}
	return os.Rename(temp, path)
}
