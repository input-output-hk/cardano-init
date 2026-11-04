package giftcard

import (
	"regexp"
	"strings"
	"testing"
)

// These need no wallet or network: script parameterisation and address
// derivation are pure. They run once the on-chain component has been built
// and `just test` has bundled its blueprint.

// Any well-formed seed works offline.
var testSeed = Seed{TxHash: strings.Repeat("00", 32), OutputIndex: 0}

func requireBlueprint(t *testing.T) {
	t.Helper()
	if !HasBundledBlueprint() {
		t.Skip("no blueprint bundled; build the on-chain component first")
	}
}

func TestBundlesBothValidators(t *testing.T) {
	requireBlueprint(t)
	v, err := BundledValidators()
	if err != nil {
		t.Fatal(err)
	}
	if len(v.GiftCard) == 0 || len(v.Redeem) == 0 {
		t.Fatal("expected both validators to be non-empty")
	}
}

func TestDerivesPolicyIDAndRedeemAddress(t *testing.T) {
	requireBlueprint(t)
	c, err := New(nil, 0) // no chain context needed for Scripts
	if err != nil {
		t.Fatal(err)
	}
	s, err := c.Scripts("hello world", testSeed)
	if err != nil {
		t.Fatal(err)
	}
	// A policy id is a 28-byte blake2b-224 hash: 56 hex chars.
	if !regexp.MustCompile(`^[0-9a-f]{56}$`).MatchString(s.PolicyID) {
		t.Errorf("policy id %q is not 56 hex chars", s.PolicyID)
	}
	if addr := s.RedeemAddress.String(); !strings.HasPrefix(addr, "addr_test") {
		t.Errorf("redeem address %q is not a testnet address", addr)
	}
	if want := s.PolicyID + "68656c6c6f20776f726c64"; s.Unit != want {
		t.Errorf("unit = %q, want %q", s.Unit, want)
	}
}

func TestDifferentSeedsGiveDifferentPolicies(t *testing.T) {
	requireBlueprint(t)
	c, err := New(nil, 0)
	if err != nil {
		t.Fatal(err)
	}
	a, err := c.Scripts("gift", testSeed)
	if err != nil {
		t.Fatal(err)
	}
	b, err := c.Scripts("gift", Seed{TxHash: testSeed.TxHash, OutputIndex: 1})
	if err != nil {
		t.Fatal(err)
	}
	if a.PolicyID == b.PolicyID {
		t.Error("the one-shot policy id should depend on the seed UTxO")
	}
}
