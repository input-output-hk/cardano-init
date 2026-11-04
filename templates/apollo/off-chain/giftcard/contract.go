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
// chain context. The config package wires one up from ../.env.
package giftcard

import (
	"encoding/hex"
	"errors"
	"fmt"
	"math/big"

	apollo "github.com/Salvionied/apollo/v2"
	"github.com/Salvionied/apollo/v2/backend"
	"github.com/blinklabs-io/gouroboros/ledger/common"
	"github.com/blinklabs-io/plutigo/data"
)

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
var (
	mintRedeemer  = common.Datum{Data: data.NewConstr(0)}
	burnRedeemer  = common.Datum{Data: data.NewConstr(1)}
	spendRedeemer = common.Datum{Data: data.NewConstr(0)} // ignored by the validator
)

func (c *Contract) giftCardScript(tokenName []byte, seedTxHash []byte, seedIndex uint32) (common.PlutusV3Script, error) {
	outRef := data.NewConstr(0,
		data.NewByteString(seedTxHash),
		data.NewInteger(new(big.Int).SetUint64(uint64(seedIndex))),
	)
	return apollo.ApplyParamsToScript(c.validators.GiftCard, data.NewByteString(tokenName), outRef)
}

func (c *Contract) redeemScript(tokenName []byte, policyID []byte) (common.PlutusV3Script, error) {
	return apollo.ApplyParamsToScript(c.validators.Redeem, data.NewByteString(tokenName), data.NewByteString(policyID))
}

func (c *Contract) scripts(tokenName []byte, seedTxHash []byte, seedIndex uint32) (Scripts, error) {
	giftCard, err := c.giftCardScript(tokenName, seedTxHash, seedIndex)
	if err != nil {
		return Scripts{}, fmt.Errorf("apply gift_card params: %w", err)
	}
	policy := giftCard.Hash()
	redeem, err := c.redeemScript(tokenName, policy.Bytes())
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
func (c *Contract) Create(walletAddr common.Address, tokenName string, giftLovelace int64) (Creation, error) {
	utxos, err := c.cc.Utxos(walletAddr)
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
	datum := common.Datum{Data: data.NewConstr(0,
		data.NewByteString(seedTxHash),
		data.NewInteger(new(big.Int).SetUint64(uint64(seedIndex))),
		data.NewByteString([]byte(tokenName)),
	)}

	built, err := apollo.New(c.cc).
		SetWallet(apollo.NewExternalWallet(walletAddr)). // watch-only: builds, never signs
		AddLoadedUTxOs(utxos...).
		AddInput(seed).
		AttachScript(s.GiftCard).
		Mint(token, &mintRedeemer, nil).
		PayToContract(s.RedeemAddress, &datum, giftLovelace, token).
		Complete()
	if err != nil {
		return Creation{}, fmt.Errorf("build create tx: %w", err)
	}
	txCbor, err := built.GetTxCbor()
	if err != nil {
		return Creation{}, err
	}
	return Creation{
		TxCbor:        hex.EncodeToString(txCbor),
		RedeemAddress: s.RedeemAddress.String(),
		Unit:          s.Unit,
	}, nil
}

// UtxoAt returns the gift-card UTxO locked at redeemAddress (the output that
// carries an inline datum), or nil if there is none yet.
func (c *Contract) UtxoAt(redeemAddress string) (*common.Utxo, error) {
	addr, err := common.NewAddress(redeemAddress)
	if err != nil {
		return nil, fmt.Errorf("redeem address: %w", err)
	}
	utxos, err := c.cc.Utxos(addr)
	if err != nil {
		return nil, fmt.Errorf("query %s: %w", redeemAddress, err)
	}
	for i := range utxos {
		if d := utxos[i].Output.Datum(); d != nil && d.Data != nil {
			return &utxos[i], nil
		}
	}
	return nil, nil
}

// Redeem builds a transaction that burns the gift-card token held by
// giftCard (see UtxoAt) and releases the locked assets to walletAddr, which
// also pays the fee and provides collateral. The transaction is returned
// UNSIGNED.
func (c *Contract) Redeem(walletAddr common.Address, giftCard common.Utxo) (string, error) {
	seedTxHash, seedIndex, tokenName, err := decodeDatum(giftCard.Output.Datum())
	if err != nil {
		return "", err
	}
	s, err := c.scripts(tokenName, seedTxHash, seedIndex)
	if err != nil {
		return "", err
	}
	utxos, err := c.cc.Utxos(walletAddr)
	if err != nil {
		return "", fmt.Errorf("query wallet UTxOs: %w", err)
	}

	built, err := apollo.New(c.cc).
		SetWallet(apollo.NewExternalWallet(walletAddr)). // watch-only: builds, never signs
		AddLoadedUTxOs(utxos...).
		CollectFrom(giftCard, spendRedeemer, common.ExUnits{}).
		AttachScript(s.Redeem).
		Mint(apollo.NewUnit(s.PolicyID, s.TokenNameHex, -1), &burnRedeemer, nil).
		AttachScript(s.GiftCard).
		Complete()
	if err != nil {
		return "", fmt.Errorf("build redeem tx: %w", err)
	}
	txCbor, err := built.GetTxCbor()
	if err != nil {
		return "", err
	}
	return hex.EncodeToString(txCbor), nil
}

// decodeDatum reads the Constr 0 [seed tx hash, seed index, token name] datum
// written by Create.
func decodeDatum(d *common.Datum) (seedTxHash []byte, seedIndex uint32, tokenName []byte, err error) {
	if d == nil || d.Data == nil {
		return nil, 0, nil, errors.New("gift-card UTxO has no inline datum")
	}
	var c *data.Constr
	switch v := d.Data.(type) {
	case *data.Constr:
		c = v
	case data.Constr:
		c = &v
	}
	if c == nil || len(c.Fields) != 3 {
		return nil, 0, nil, errors.New("unexpected gift-card datum shape")
	}
	txHash, ok1 := asBytes(c.Fields[0])
	index, ok2 := asInt(c.Fields[1])
	name, ok3 := asBytes(c.Fields[2])
	if !ok1 || !ok2 || !ok3 || !index.IsUint64() || index.Uint64() > uint64(^uint32(0)) {
		return nil, 0, nil, errors.New("unexpected gift-card datum fields")
	}
	return txHash, uint32(index.Uint64()), name, nil
}

func asBytes(pd data.PlutusData) ([]byte, bool) {
	switch v := pd.(type) {
	case *data.ByteString:
		return v.Inner, true
	case data.ByteString:
		return v.Inner, true
	}
	return nil, false
}

func asInt(pd data.PlutusData) (*big.Int, bool) {
	switch v := pd.(type) {
	case *data.Integer:
		return v.Inner, v.Inner != nil
	case data.Integer:
		return v.Inner, v.Inner != nil
	}
	return nil, false
}

// SignAndSubmit signs an unsigned transaction with wallet and submits it
// through the contract's chain context. It returns the transaction hash.
func (c *Contract) SignAndSubmit(wallet apollo.Wallet, unsignedTx string) (string, error) {
	b, err := apollo.New(c.cc).SetWallet(wallet).LoadTxCbor(unsignedTx)
	if err != nil {
		return "", err
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
