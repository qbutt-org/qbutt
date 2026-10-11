package main

import (
	"context"
	"flag"
	"fmt"
	"os"

	"qbutt.org/search-engine/internal/app"
)

func main() {
	dir := flag.String("data-dir", "", "Private qbutt search data directory")
	flag.Parse()
	if *dir == "" {
		fmt.Fprintln(os.Stderr, "--data-dir is required")
		os.Exit(2)
	}
	engine, err := app.New(*dir)
	if err != nil {
		fmt.Fprintln(os.Stderr, "Search data could not be opened")
		os.Exit(1)
	}
	if err := engine.Serve(context.Background(), os.Stdin, os.Stdout); err != nil {
		fmt.Fprintln(os.Stderr, "Search control channel failed")
		os.Exit(1)
	}
}
