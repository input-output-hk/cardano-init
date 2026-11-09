// Package giftcard holds the off-chain bindings for the GiftCard contract,
// built on Apollo. It derives the parameterised scripts from the on-chain
// blueprint and builds the two transactions of the flow:
//
//   - Create mints a one-shot gift-card token and locks it, with some
//     lovelace, at the redeem script address.
//   - Redeem burns the token and releases the locked assets.
//
// Both return UNSIGNED transactions, so a backend can build for a user's
// address and leave signing to the user's wallet. SignAndSubmit signs with a
// local wallet (for scripts, tests, and the bundled CLI).
//
// The package reads no environment and opens no files: give it any Apollo
// chain context. The internal/config package wires one up from ../.env.
package giftcard

import (
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	"math/big"

	apollo "github.com/Salvionied/apollo/v2"
	"github.com/Salvionied/apollo/v2/backend"
	"github.com/blinklabs-io/gouroboros/ledger/common"
	"github.com/blinklabs-io/plutigo/data"
)

// ErrNoGiftCard is returned by UtxoAt when no gift card is locked at the
// address (not created yet, not indexed yet, or already redeemed).
var ErrNoGiftCard = errors.New("no gift-card UTxO at this address")

// Validators are the two unparameterised validators of the GiftCard flow.
type Validators struct {
	// GiftCard is the one-shot minting policy.
	GiftCard common.PlutusV3Script
	// Redeem is the spending validator guarding the locked gift.
	Redeem common.PlutusV3Script
}

// ValidatorsFromBlueprint pulls the gift_card (mint) and redeem (spend)
// validators out of a blueprint.
func ValidatorsFromBlueprint(bp *Blueprint) (Validators, error) {
	giftCard, err := scriptFromBlueprint(bp, "gift_card.mint")
	if err != nil {
		return Validators{}, err
	}
	redeem, err := scriptFromBlueprint(bp, "redeem.spend")
	if err != nil {
		return Validators{}, err
	}
	return Validators{GiftCard: giftCard, Redeem: redeem}, nil
}

func scriptFromBlueprint(bp *Blueprint, suffix string) (common.PlutusV3Script, error) {
	code, err := bp.FindValidator(suffix)
	if err != nil {
		return nil, err
	}
	raw, err := hex.DecodeString(code)
	if err != nil {
		return nil, fmt.Errorf("validator %q: invalid compiledCode hex: %w", suffix, err)
	}
	return common.PlutusV3Script(raw), nil
}

// BundledValidators returns the validators from the blueprint embedded at
// build time.
func BundledValidators() (Validators, error) {
	bp, err := BundledBlueprint()
	if err != nil {
		return Validators{}, err
	}
	return ValidatorsFromBlueprint(bp)
}

// Seed references the UTxO that parameterises a gift card. Spending it in the
// create transaction is what makes the minting policy one-shot.
type Seed struct {
	TxHash      string
	OutputIndex uint32
}

// Scripts are the parameterised scripts and derived identifiers of one gift
// card.
type Scripts struct {
	GiftCard      common.PlutusV3Script
	PolicyID      string
	Redeem        common.PlutusV3Script
	RedeemAddress common.Address
	// TokenNameHex is the token name as hex, the form Apollo units use.
	TokenNameHex string
	// Unit is the asset unit of the gift-card token: policy id + hex name.
	Unit string
}

// Creation is the result of Create.
type Creation struct {
	// TxCbor is the unsigned create transaction as CBOR hex.
	TxCbor string
	// RedeemAddress is the bech32 script address the gift is locked at. Each
	// gift card has its own address, so it identifies the gift card.
	RedeemAddress string
	// Unit is the asset unit of the minted gift-card token.
	Unit string
}

// Contract builds GiftCard transactions against a chain context.
type Contract struct {
	cc         backend.ChainContext
	networkID  uint8
	validators Validators
}

// New returns a Contract that uses the blueprint embedded at build time.
// networkID is 0 for testnets (preview, preprod, a local devnet) and 1 for
// mainnet. cc may be nil if you only call Scripts.
func New(cc backend.ChainContext, networkID uint8) (*Contract, error) {
	v, err := BundledValidators()
	if err != nil {
		return nil, err
	}
	return NewWithValidators(cc, networkID, v), nil
}

// NewWithValidators returns a Contract that uses the given validators instead
// of the bundled blueprint.
func NewWithValidators(cc backend.ChainContext, networkID uint8, v Validators) *Contract {
	return &Contract{cc: cc, networkID: networkID, validators: v}
}

// Wire format shared by every on-chain template (see the gift-card spec):
//
//	gift_card params: token_name (bytes), seed OutputReference
//	redeem params:    token_name (bytes), gift_card policy id (bytes)
//	OutputReference:  Constr 0 [tx_id (bare bytes), output_index]
//	mint redeemer:    Mint = Constr 0 [], Burn = Constr 1 []
//	datum:            Constr 0 [seed tx_id, seed output_index, token_name]
//
// The validator ignores the spend redeemer; Constr 0 [] is a placeholder.
// Redeemers are built per call so no transaction shares Plutus data values.

func mintRedeemer() *common.Datum { return &common.Datum{Data: data.NewConstr(0)} }
func burnRedeemer() *common.Datum { return &common.Datum{Data: data.NewConstr(1)} }
func spendRedeemer() common.Datum { return common.Datum{Data: data.NewConstr(0)} }

func uintData(n uint32) data.PlutusData {
	return data.NewInteger(new(big.Int).SetUint64(uint64(n)))
}

func outputReference(txHash []byte, index uint32) data.PlutusData {
	return data.NewConstr(0, data.NewByteString(txHash), uintData(index))
}

func giftCardDatum(seedTxHash []byte, seedIndex uint32, tokenName []byte) *common.Datum {
	return &common.Datum{Data: data.NewConstr(0,
		data.NewByteString(seedTxHash),
		uintData(seedIndex),
		data.NewByteString(tokenName),
	)}
}

func (c *Contract) scripts(tokenName, seedTxHash []byte, seedIndex uint32) (Scripts, error) {
	giftCard, err := apollo.ApplyParamsToScript(c.validators.GiftCard,
		data.NewByteString(tokenName), outputReference(seedTxHash, seedIndex))
	if err != nil {
		return Scripts{}, fmt.Errorf("apply gift_card params: %w", err)
	}
	policy := giftCard.Hash()
	redeem, err := apollo.ApplyParamsToScript(c.validators.Redeem,
		data.NewByteString(tokenName), data.NewByteString(policy.Bytes()))
	if err != nil {
		return Scripts{}, fmt.Errorf("apply redeem params: %w", err)
	}
	redeemHash := redeem.Hash()
	addr, err := common.NewAddressFromParts(common.AddressTypeScriptNone, c.networkID, redeemHash.Bytes(), nil)
	if err != nil {
		return Scripts{}, fmt.Errorf("redeem address: %w", err)
	}
	nameHex := hex.EncodeToString(tokenName)
	return Scripts{
		GiftCard:      giftCard,
		PolicyID:      policy.String(),
		Redeem:        redeem,
		RedeemAddress: addr,
		TokenNameHex:  nameHex,
		Unit:          policy.String() + nameHex,
	}, nil
}

// Scripts derives the parameterised scripts, policy id, and redeem address of
// the gift card named tokenName and seeded by seed. It needs no chain access.
func (c *Contract) Scripts(tokenName string, seed Seed) (Scripts, error) {
	txHash, err := hex.DecodeString(seed.TxHash)
	if err != nil {
		return Scripts{}, fmt.Errorf("seed tx hash: %w", err)
	}
	return c.scripts([]byte(tokenName), txHash, seed.OutputIndex)
}

// Create builds a transaction that mints a gift-card token and locks it, with
// giftLovelace, at the redeem script address. walletAddr funds the
// transaction and receives the change; one of its UTxOs becomes the seed. The
// transaction is returned UNSIGNED.
func (c *Contract) Create(ctx context.Context, walletAddr common.Address, tokenName string, giftLovelace int64) (Creation, error) {
	if giftLovelace <= 0 {
		return Creation{}, fmt.Errorf("gift amount must be positive, got %d lovelace", giftLovelace)
	}
	utxos, err := backend.UtxosContext(ctx, c.cc, walletAddr)
	if err != nil {
		return Creation{}, fmt.Errorf("query wallet UTxOs: %w", err)
	}
	if len(utxos) == 0 {
		return Creation{}, fmt.Errorf("no UTxOs at %s; fund the wallet first", walletAddr)
	}
	seed := utxos[0]
	seedTxHash := seed.Id.Id().Bytes()
	seedIndex := seed.Id.Index()

	s, err := c.scripts([]byte(tokenName), seedTxHash, seedIndex)
	if err != nil {
		return Creation{}, err
	}
	token := apollo.NewUnit(s.PolicyID, s.TokenNameHex, 1)

	// Keep the seed reference and token name in the inline datum, so Redeem
	// can rebuild the parameterised scripts from the on-chain UTxO alone. The
	// validator ignores the datum; burning the token is the authorisation.
	datum := giftCardDatum(seedTxHash, seedIndex, []byte(tokenName))

	built, err := apollo.New(c.cc).
		WithContext(ctx).
		SetWallet(apollo.NewExternalWallet(walletAddr)). // watch-only: builds, never signs
		AddLoadedUTxOs(utxos...).
		AddInput(seed).
		AttachScript(s.GiftCard).
		Mint(token, mintRedeemer(), nil).
		PayToContract(s.RedeemAddress, datum, giftLovelace, token).
		Complete()
	if err != nil {
		return Creation{}, fmt.Errorf("build create tx: %w", err)
	}
	txCbor, err := built.GetTxCbor()
	if err != nil {
		return Creation{}, fmt.Errorf("encode create tx: %w", err)
	}
	return Creation{
		TxCbor:        hex.EncodeToString(txCbor),
		RedeemAddress: s.RedeemAddress.String(),
		Unit:          s.Unit,
	}, nil
}

// UtxoAt returns the gift-card UTxO locked at redeemAddress. It returns
// ErrNoGiftCard if there is none.
//
// Anyone can pay to a script address, with any datum, so outputs are checked
// rather than trusted: see isGiftCard. Unrelated outputs are skipped.
func (c *Contract) UtxoAt(ctx context.Context, redeemAddress string) (common.Utxo, error) {
	addr, err := common.NewAddress(redeemAddress)
	if err != nil {
		return common.Utxo{}, fmt.Errorf("redeem address: %w", err)
	}
	utxos, err := backend.UtxosContext(ctx, c.cc, addr)
	if err != nil {
		return common.Utxo{}, fmt.Errorf("query %s: %w", redeemAddress, err)
	}
	for _, u := range utxos {
		if c.isGiftCard(addr, u.Output.Datum(), u.Output.Assets()) {
			return u, nil
		}
	}
	return common.Utxo{}, ErrNoGiftCard
}

// isGiftCard reports whether an output at addr, with datum d and assets, is a
// genuine gift card: the datum decodes, the scripts it parameterises derive
// back to addr, and the output holds exactly one of that gift card's token.
func (c *Contract) isGiftCard(addr common.Address, d *common.Datum, assets *common.MultiAsset[common.MultiAssetTypeOutput]) bool {
	seedTxHash, seedIndex, tokenName, err := decodeDatum(d)
	if err != nil || assets == nil {
		return false
	}
	s, err := c.scripts(tokenName, seedTxHash, seedIndex)
	if err != nil || s.RedeemAddress.String() != addr.String() {
		return false
	}
	n := assets.Asset(s.GiftCard.Hash(), tokenName)
	return n != nil && n.Cmp(big.NewInt(1)) == 0
}

// Redeem builds a transaction that burns the gift-card token held by
// giftCard (see UtxoAt) and releases the locked assets to walletAddr, which
// also pays the fee and provides collateral. The transaction is returned
// UNSIGNED, as CBOR hex.
func (c *Contract) Redeem(ctx context.Context, walletAddr common.Address, giftCard common.Utxo) (string, error) {
	seedTxHash, seedIndex, tokenName, err := decodeDatum(giftCard.Output.Datum())
	if err != nil {
		return "", err
	}
	s, err := c.scripts(tokenName, seedTxHash, seedIndex)
	if err != nil {
		return "", err
	}
	utxos, err := backend.UtxosContext(ctx, c.cc, walletAddr)
	if err != nil {
		return "", fmt.Errorf("query wallet UTxOs: %w", err)
	}

	built, err := apollo.New(c.cc).
		WithContext(ctx).
		SetWallet(apollo.NewExternalWallet(walletAddr)). // watch-only: builds, never signs
		AddLoadedUTxOs(utxos...).
		CollectFrom(giftCard, spendRedeemer(), common.ExUnits{}).
		AttachScript(s.Redeem).
		Mint(apollo.NewUnit(s.PolicyID, s.TokenNameHex, -1), burnRedeemer(), nil).
		AttachScript(s.GiftCard).
		Complete()
	if err != nil {
		return "", fmt.Errorf("build redeem tx: %w", err)
	}
	txCbor, err := built.GetTxCbor()
	if err != nil {
		return "", fmt.Errorf("encode redeem tx: %w", err)
	}
	return hex.EncodeToString(txCbor), nil
}

// decodeDatum reads the datum written by Create (see giftCardDatum). plutigo
// decodes Plutus data to pointer types.
func decodeDatum(d *common.Datum) (seedTxHash []byte, seedIndex uint32, tokenName []byte, err error) {
	if d == nil || d.Data == nil {
		return nil, 0, nil, errors.New("gift-card UTxO has no inline datum")
	}
	c, ok := d.Data.(*data.Constr)
	if !ok || c.Tag == nil || c.Tag.Sign() != 0 || len(c.Fields) != 3 {
		return nil, 0, nil, errors.New("unexpected gift-card datum shape")
	}
	txHash, ok1 := c.Fields[0].(*data.ByteString)
	index, ok2 := c.Fields[1].(*data.Integer)
	name, ok3 := c.Fields[2].(*data.ByteString)
	if !ok1 || !ok2 || !ok3 || index.Inner == nil || !index.Inner.IsUint64() || index.Inner.Uint64() > uint64(^uint32(0)) {
		return nil, 0, nil, errors.New("unexpected gift-card datum fields")
	}
	return txHash.Inner, uint32(index.Inner.Uint64()), name.Inner, nil
}

// SignAndSubmit signs an unsigned transaction (CBOR hex) with wallet and
// submits it through the contract's chain context. It returns the
// transaction hash as hex.
func (c *Contract) SignAndSubmit(ctx context.Context, wallet apollo.Wallet, unsignedTx string) (string, error) {
	b, err := apollo.New(c.cc).WithContext(ctx).SetWallet(wallet).LoadTxCbor(unsignedTx)
	if err != nil {
		return "", fmt.Errorf("load tx: %w", err)
	}
	if b, err = b.Sign(); err != nil {
		return "", fmt.Errorf("sign: %w", err)
	}
	txID, err := b.Submit()
	if err != nil {
		return "", fmt.Errorf("submit: %w", err)
	}
	return hex.EncodeToString(txID.Bytes()), nil
}
