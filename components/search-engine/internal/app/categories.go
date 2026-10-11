package app

import (
	"maps"
	"slices"

	"qbutt.org/search-engine/internal/indexer/cardigann/mapper"
)

func canonicalCategory(id int) string {
	switch {
	case id == 5070:
		return "anime"
	case id == 4050 || id >= 1000 && id < 2000:
		return "games"
	case id >= 2000 && id < 3000:
		return "movies"
	case id >= 5000 && id < 6000:
		return "tv"
	case id >= 4000 && id < 5000:
		return "software"
	case id >= 7000 && id < 8000:
		return "books"
	default:
		return "all"
	}
}

func categories(caps *mapper.Capabilities) []string {
	set := map[string]bool{}
	for _, category := range caps.Categories {
		if category.IsCustom() {
			continue
		}
		set[canonicalCategory(category.ID)] = true
	}
	if len(set) == 0 {
		return []string{"all"}
	}
	return slices.Sorted(maps.Keys(set))
}

func categoryIDs(category string) []int {
	switch category {
	case "games":
		return []int{1000, 4050}
	case "movies":
		return []int{2000}
	case "tv":
		return []int{5000}
	case "anime":
		return []int{5070}
	case "software":
		return []int{4000}
	case "books":
		return []int{7000}
	case "manga":
		return []int{7030}
	default:
		return nil
	}
}

func queryCategories(caps *mapper.Capabilities, category string) ([]string, bool) {
	if category == "" || category == "all" {
		return caps.DefaultCategories, true
	}
	mapped := caps.MapTorznabCapsToTrackers(categoryIDs(category))
	return mapped, len(mapped) > 0
}

func releaseCategory(ids []int, requested string) string {
	actual := map[string]bool{}
	for _, id := range ids {
		category := canonicalCategory(id)
		if category != "all" {
			actual[category] = true
		}
	}
	// Some definitions map one tracker category to multiple standard families.
	// Prefer the requested family only when the actual release carries it.
	if actual[requested] {
		return requested
	}
	for _, category := range []string{"anime", "games", "books", "movies", "tv", "software"} {
		if actual[category] {
			return category
		}
	}
	return "all"
}

func releaseMatchesCategory(ids []int, requested string) bool {
	if requested == "" || requested == "all" || releaseCategory(ids, requested) == requested {
		return true
	}
	for _, id := range ids {
		if id >= 1000 && id < 8000 {
			// A known different family includes unsupported media such as Audio.
			return false
		}
	}
	// Other and missing categories stay unknown, never relabelled as requested.
	return true
}
