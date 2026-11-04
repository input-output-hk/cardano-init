package config

import (
	"errors"
	"os"
	"path/filepath"
	"slices"
	"testing"
)

func TestResolveProvider(t *testing.T) {
	tests := []struct {
		name string
		vars map[string]string
		want Provider
	}{
		{
			name: "yaci devnet",
			vars: map[string]string{"INDEXER_URL": "http://localhost:8080/api/v1/", "YACI_ADMIN_URL": "http://localhost:10000"},
			want: Provider{Kind: Indexer, URL: "http://localhost:8080/api/v1/", AdminURL: "http://localhost:10000"},
		},
		{
			name: "blockfrost-compatible indexer alone (Dingo)",
			vars: map[string]string{"INDEXER_URL": "http://localhost:3000/api/v0"},
			want: Provider{Kind: Indexer, URL: "http://localhost:3000/api/v0"},
		},
		{
			name: "ogmios with kupo",
			vars: map[string]string{"OGMIOS_URL": "ws://localhost:1337", "INDEXER_URL": "http://localhost:1442"},
			want: Provider{Kind: Kupmios, URL: "ws://localhost:1337", KupoURL: "http://localhost:1442"},
		},
		{
			name: "dolos",
			vars: map[string]string{"DOLOS_GRPC_URL": "http://localhost:50051"},
			want: Provider{Kind: UtxoRPC, URL: "http://localhost:50051"},
		},
		{
			name: "public blockfrost",
			vars: map[string]string{"BLOCKFROST_PROJECT_ID": "preview123"},
			want: Provider{Kind: Blockfrost, URL: blockfrostURLs["preview"], ProjectID: "preview123"},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			tt.vars["MNEMONIC"] = "words"
			env, err := Resolve(mapLookup(tt.vars))
			if err != nil {
				t.Fatal(err)
			}
			if env.Provider != tt.want {
				t.Errorf("provider = %+v, want %+v", env.Provider, tt.want)
			}
		})
	}
}

func TestResolveNetwork(t *testing.T) {
	tests := []struct {
		network   string
		wantName  string
		wantID    uint8
		wantError bool
	}{
		{network: "", wantName: "preview", wantID: 0},
		{network: "preprod", wantName: "preprod", wantID: 0},
		{network: "mainnet", wantName: "mainnet", wantID: 1},
		{network: "sanchonet", wantError: true},
	}
	for _, tt := range tests {
		t.Run("network="+tt.network, func(t *testing.T) {
			env, err := Resolve(mapLookup(map[string]string{
				"CARDANO_NETWORK": tt.network, "INDEXER_URL": "http://indexer", "MNEMONIC": "words",
			}))
			if tt.wantError {
				var missing *MissingError
				if !errors.As(err, &missing) {
					t.Fatalf("err = %v, want a *MissingError", err)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if env.Network != tt.wantName || env.NetworkID != tt.wantID {
				t.Errorf("network = %s/%d, want %s/%d", env.Network, env.NetworkID, tt.wantName, tt.wantID)
			}
		})
	}
}

func TestResolveReportsEveryMissingSetting(t *testing.T) {
	_, err := Resolve(mapLookup(nil))
	var missing *MissingError
	if !errors.As(err, &missing) {
		t.Fatalf("err = %v, want a *MissingError", err)
	}
	want := []string{"BLOCKFROST_PROJECT_ID (or start a local devnet: INDEXER_URL)", "MNEMONIC"}
	if !slices.Equal(missing.Settings, want) {
		t.Errorf("missing = %q, want %q", missing.Settings, want)
	}
}

func TestLookupPrecedence(t *testing.T) {
	root := t.TempDir()
	dir := filepath.Join(root, "off-chain")
	if err := os.Mkdir(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	write := func(path, content string) {
		t.Helper()
		if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	write(filepath.Join(root, ".env"), "CARDANO_NETWORK=preprod\nINDEXER_URL=http://shared # set by the devnet\nMNEMONIC=\n")
	write(filepath.Join(dir, ".env.local"), "MNEMONIC=\"from local\"\n")
	t.Setenv("INDEXER_URL", "http://process")
	t.Setenv("CARDANO_NETWORK", "") // blank in the process env: the file still applies

	get, err := Lookup(dir)
	if err != nil {
		t.Fatal(err)
	}
	for key, want := range map[string]string{
		"CARDANO_NETWORK": "preprod",        // shared ../.env
		"MNEMONIC":        "from local",     // .env.local overrides ../.env
		"INDEXER_URL":     "http://process", // process environment wins
		"OGMIOS_URL":      "",               // set nowhere
	} {
		if got := get(key); got != want {
			t.Errorf("%s = %q, want %q", key, got, want)
		}
	}
}

func TestLookupWithoutFiles(t *testing.T) {
	get, err := Lookup(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if got := get("SURELY_NOT_SET_ANYWHERE"); got != "" {
		t.Errorf("got %q, want empty", got)
	}
}

func mapLookup(vars map[string]string) func(string) string {
	return func(key string) string { return vars[key] }
}
