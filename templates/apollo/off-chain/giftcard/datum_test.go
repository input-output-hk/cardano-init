package giftcard

import (
	"bytes"
	"testing"

	"github.com/blinklabs-io/gouroboros/ledger/common"
	"github.com/blinklabs-io/plutigo/data"
)

// The datum Create writes must decode back to the same seed and token name
// after a CBOR round-trip, which is how Redeem receives it from the chain.
func TestDatumRoundTrip(t *testing.T) {
	txHash := bytes.Repeat([]byte{0xab}, 32)
	written := giftCardDatum(txHash, 7, []byte("gift"))

	raw, err := data.Encode(written.Data)
	if err != nil {
		t.Fatal(err)
	}
	var onChain common.Datum
	if err := onChain.UnmarshalCBOR(raw); err != nil {
		t.Fatal(err)
	}

	gotHash, gotIndex, gotName, err := decodeDatum(&onChain)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(gotHash, txHash) || gotIndex != 7 || string(gotName) != "gift" {
		t.Errorf("decoded (%x, %d, %q), want (%x, 7, \"gift\")", gotHash, gotIndex, gotName, txHash)
	}
}

func TestDecodeDatumRejectsOtherShapes(t *testing.T) {
	tests := []struct {
		name  string
		datum *common.Datum
	}{
		{"nil datum", nil},
		{"empty datum", &common.Datum{}},
		{"not a constructor", &common.Datum{Data: data.NewByteString([]byte("x"))}},
		{"wrong tag", &common.Datum{Data: data.NewConstr(1,
			data.NewByteString(nil), uintData(0), data.NewByteString(nil))}},
		{"too few fields", &common.Datum{Data: data.NewConstr(0, data.NewByteString(nil))}},
		{"index is bytes", &common.Datum{Data: data.NewConstr(0,
			data.NewByteString(nil), data.NewByteString(nil), data.NewByteString(nil))}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if _, _, _, err := decodeDatum(tt.datum); err == nil {
				t.Error("expected an error")
			}
		})
	}
}
