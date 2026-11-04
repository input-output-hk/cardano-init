package giftcard

import (
	"embed"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"strings"
)

// The on-chain blueprint is embedded at build time: `just build` (and `just
// test`) copy ../blueprint/plutus.json to bundle/plutus.json first. Importers
// of this package get the validators with it and never supply plutus.json.
//
// The directory is embedded rather than the file so the package still compiles
// before the on-chain component has been built (bundle/ then holds only
// .gitkeep, and BundledBlueprint reports that nothing was bundled).
//
//go:embed all:bundle
var bundle embed.FS

const bundledPath = "bundle/plutus.json"

// Blueprint is the part of a CIP-57 plutus.json this package reads. The rest
// of the document (preamble details, datum and redeemer schemas) is ignored,
// so blueprints from any on-chain tool load the same way.
type Blueprint struct {
	Validators []Validator `json:"validators"`
}

// Validator is one compiled validator in a blueprint.
type Validator struct {
	Title string `json:"title"`
	// CompiledCode is the CBOR-wrapped flat UPLC program, hex encoded.
	CompiledCode string `json:"compiledCode"`
}

// ParseBlueprint decodes a CIP-57 plutus.json document.
func ParseBlueprint(raw []byte) (*Blueprint, error) {
	var bp Blueprint
	if err := json.Unmarshal(raw, &bp); err != nil {
		return nil, fmt.Errorf("parse blueprint: %w", err)
	}
	return &bp, nil
}

// FindValidator returns the compiled code of the validator whose title ends in
// "<validator>.<purpose>" (for example "gift_card.mint"). Matching on the
// suffix keeps the lookup independent of the on-chain module name.
func (bp *Blueprint) FindValidator(suffix string) (string, error) {
	titles := make([]string, 0, len(bp.Validators))
	for _, v := range bp.Validators {
		if v.Title == suffix || strings.HasSuffix(v.Title, "."+suffix) {
			return v.CompiledCode, nil
		}
		titles = append(titles, v.Title)
	}
	return "", fmt.Errorf("validator %q not found in blueprint; available: %s",
		suffix, strings.Join(titles, ", "))
}

// ErrNoBundledBlueprint is returned when the build carries no blueprint.
var ErrNoBundledBlueprint = errors.New(
	"no blueprint was bundled into this build: build the on-chain component " +
		"(its `just build` writes ../blueprint/plutus.json), then run the " +
		"off-chain `just build`; or pass validators to NewWithValidators")

// BundledBlueprint returns the blueprint embedded at build time, or
// ErrNoBundledBlueprint if none was bundled.
func BundledBlueprint() (*Blueprint, error) {
	raw, err := bundle.ReadFile(bundledPath)
	if errors.Is(err, fs.ErrNotExist) {
		return nil, ErrNoBundledBlueprint
	}
	if err != nil {
		return nil, err
	}
	return ParseBlueprint(raw)
}

// HasBundledBlueprint reports whether a blueprint was embedded at build time.
func HasBundledBlueprint() bool {
	_, err := bundle.ReadFile(bundledPath)
	return err == nil
}
