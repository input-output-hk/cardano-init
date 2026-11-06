package app

import org.scalatest.funsuite.AnyFunSuite
import scalus.cardano.address.Address
import scalus.crypto.ed25519.given
import scalus.utils.await

import java.net.URI
import java.net.http.{HttpClient, HttpRequest, HttpResponse}
import scala.concurrent.ExecutionContext.Implicits.global
import scala.concurrent.duration.*

/** Creates and redeems a gift card on a live Yaci devnet. Skipped when none is running. */
class TransactionsDevnetTest extends AnyFunSuite {
    private val env = DotEnv.load()
    private val adminUrl = env.get("YACI_ADMIN_URL").map(_.trim).filter(_.nonEmpty)
    private val requireDevnet = env.get("CARDANO_INIT_REQUIRE_DEVNET").exists(_.trim.nonEmpty)

    test("a gift card can be created and redeemed on the devnet") {
        assume(adminUrl.isDefined, "no Yaci devnet is running")
        assume(requireDevnet || GiftCardScripts.blueprintExists(), "build an on-chain component first")
        val appCtx = AppCtx.yaciDevKit("Gift Card")
        val txBuilder = Transactions(appCtx, GiftCardScripts.fromFile())

        // Yaci's faucet takes ADA. One small UTxO for collateral and a large one to spend.
        topup(appCtx.address.encode.get, 10)
        topup(appCtx.address.encode.get, 10000)
        eventually("the faucet funds to be indexed") {
            val utxos = appCtx.provider.findUtxos(appCtx.address).await(30.seconds)
            utxos.toOption.filter(_.size >= 2).toRight("wallet not funded yet")
        }

        val card = txBuilder.createGiftCard(5_000_000L) match
            case Right(created) => created
            case Left(err)      => fail(s"create failed: $err")

        // Redeem fails until the indexer has seen the create transaction.
        eventually("the gift card to be redeemed") {
            txBuilder.redeemGiftCard(card.seed)
        }

        val redeemAddress = Address.fromBech32(card.redeemAddress)
        eventually("the gift card to leave the redeem address") {
            val utxos = appCtx.provider.findUtxos(redeemAddress).await(30.seconds)
            Either.cond(utxos.toOption.forall(_.isEmpty), (), "still locked")
        }
    }

    private def topup(address: String, ada: Int): Unit = {
        val request = HttpRequest
            .newBuilder(URI.create(s"${adminUrl.get.stripSuffix("/")}/local-cluster/api/addresses/topup"))
            .header("Content-Type", "application/json")
            .POST(HttpRequest.BodyPublishers.ofString(s"""{"address":"$address","adaAmount":$ada}"""))
            .build()
        val response = HttpClient.newHttpClient().send(request, HttpResponse.BodyHandlers.ofString())
        assert(response.statusCode() == 200, s"faucet topup failed: ${response.body()}")
    }

    private def eventually[A](what: String)(attempt: => Either[String, A]): A = {
        val deadline = 2.minutes.fromNow
        var result = attempt
        while (result.isLeft && deadline.hasTimeLeft()) {
            Thread.sleep(2000)
            result = attempt
        }
        result.fold(err => fail(s"timed out waiting for $what: $err"), identity)
    }
}
