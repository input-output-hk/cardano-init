package giftcard

import (
	"bytes"
	"math/big"
	"testing"

	"github.com/blinklabs-io/gouroboros/cbor"
	"github.com/blinklabs-io/gouroboros/ledger/common"
	"github.com/blinklabs-io/plutigo/data"
)

// Anyone can pay to a script address with any datum, so UtxoAt must only
// accept an output that is the gift card the address was derived for.
func TestIsGiftCard(t *testing.T) {
	requireBlueprint(t)
	c, err := New(nil, 0)
	if err != nil {
		t.Fatal(err)
	}
	seedHash := bytes.Repeat([]byte{0xab}, 32)
	name := []byte("gift")
	s, err := c.scripts(name, seedHash, 3)
	if err != nil {
		t.Fatal(err)
	}
	other, err := c.scripts(name, seedHash, 4)
	if err != nil {
		t.Fatal(err)
	}
	holding := func(s Scripts, n int64) *common.MultiAsset[common.MultiAssetTypeOutput] {
		m := common.NewMultiAsset(map[common.Blake2b224]map[cbor.ByteString]*big.Int{
			s.GiftCard.Hash(): {cbor.NewByteString(name): big.NewInt(n)},
		})
		return &m
	}
	genuine := giftCardDatum(seedHash, 3, name)

	tests := []struct {
		name   string
		datum  *common.Datum
		assets *common.MultiAsset[common.MultiAssetTypeOutput]
		want   bool
	}{
		{"genuine gift card", genuine, holding(s, 1), true},
		{"no datum", nil, holding(s, 1), false},
		{"malformed datum", &common.Datum{Data: data.NewConstr(0)}, holding(s, 1), false},
		{"datum for another gift card", giftCardDatum(seedHash, 4, name), holding(other, 1), false},
		{"no assets", genuine, nil, false},
		{"missing token", genuine, holding(other, 1), false},
		{"wrong token quantity", genuine, holding(s, 2), false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := c.isGiftCard(s.RedeemAddress, tt.datum, tt.assets); got != tt.want {
				t.Errorf("isGiftCard = %v, want %v", got, tt.want)
			}
		})
	}
}
