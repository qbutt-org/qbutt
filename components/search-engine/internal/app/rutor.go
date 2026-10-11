package app

import (
	"context"
	"errors"
	"net/url"
	"regexp"
	"slices"

	"qbutt.org/search-engine/internal/indexer/cardigann"
)

// RuTor filters by one site category per request, but its result rows have no
// category field. Keep its current routing separate from the upstream selectors.
var rutorRoutes = map[string]struct {
	IDs      []string
	Category int
}{
	"movies":   {[]string{"1", "5", "12"}, 2000},
	"tv":       {[]string{"4", "16", "6"}, 5000},
	"anime":    {[]string{"10"}, 5070},
	"games":    {[]string{"8"}, 4050},
	"software": {[]string{"9"}, 4000},
	"books":    {[]string{"11"}, 7000},
}

var rutorResponseCategory = regexp.MustCompile(`\bvar\s+search_category\s*=\s*(\d+)\s*;`)

func searchReleases(ctx context.Context, source Source, i *instance, engine *cardigann.Engine, query, category string) ([]*cardigann.Release, error) {
	if source.ID != "rutor" || category == "" || category == "all" {
		categories, supported := queryCategories(engine.Capabilities(), category)
		if !supported {
			return nil, nil
		}
		return engine.Search(ctx, cardigann.Query{Keywords: query, Categories: categories})
	}
	route, supported := rutorRoutes[category]
	if !supported {
		return nil, nil
	}
	base := source.URL
	if configured := i.settings["baseUrl"]; configured != "" {
		base = configured
	}
	sort := i.settings["sort"]
	if !slices.Contains([]string{"0", "1", "2", "3", "6", "7", "8", "9"}, sort) {
		sort = "0"
	}
	var releases []*cardigann.Release
	for _, id := range route.IDs {
		address := absolute(base, "search/0/"+id+"/100/"+sort+"/"+url.PathEscape(query)+"/")
		body, _, err := fetchPage(ctx, i.client, address)
		if err != nil {
			return nil, err
		}
		// The server must acknowledge the selected category. A changed route or
		// challenge page must never turn an unfiltered response into labelled rows.
		selected := rutorResponseCategory.FindSubmatch(body)
		if len(selected) != 2 || string(selected[1]) != id {
			return nil, errors.New("RuTor did not confirm the selected category")
		}
		rows, err := engine.ParseResponseQuery(body, "", cardigann.Query{Keywords: query, Categories: []string{id}})
		if err != nil {
			return nil, err
		}
		for _, row := range rows {
			row.Categories = []int{route.Category}
			releases = append(releases, row)
		}
	}
	return releases, nil
}
