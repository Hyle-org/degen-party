import { computed, reactive } from "vue";
import { WalletContextType, WalletUpdateEvent } from "wallet-wrapper";
import { gameState } from "../game_data/game_data";
import { authService } from "../game_data/auth";

export const walletState = reactive({
    wallet: null as WalletContextType["wallet"],
    sessionKey: null as { privateKey: string; publicKey: string } | null,
    createIdentityBlobs: null as any as WalletContextType["createIdentityBlobs"],
    getOrReuseSessionKey: null as any as WalletContextType["getOrReuseSessionKey"],
});

export const onWalletReady = async (walletEvent: WalletUpdateEvent) => {
    const { wallet, getOrReuseSessionKey, createIdentityBlobs } = walletEvent.detail;
    walletState.wallet = wallet;
    walletState.getOrReuseSessionKey = getOrReuseSessionKey;
    walletState.createIdentityBlobs = createIdentityBlobs;
    const sessKey = await walletState.getOrReuseSessionKey();
    if (sessKey) {
        walletState.sessionKey = sessKey;
        authService.reload(sessKey.privateKey, sessKey.publicKey);
    } else {
        walletState.sessionKey = null;
    }
};

export const walletConfig =
    window.location.hostname === "localhost"
        ? {
              nodeBaseUrl: "http://localhost:4321",
              walletServerBaseUrl: "http://localhost:4000",
              applicationWsUrl: "ws://localhost:8081/ws",
          }
        : {
              nodeBaseUrl: "https://node.testnet.hyli.org",
              walletServerBaseUrl: "https://wallet.testnet.hyli.org",
              applicationWsUrl: "wss://wallet.testnet.hyli.org/ws",
          };

export const sessionKeyConfig = computed(() => {
    let ret = {
        duration: 60 * 60 * 24 * 7 * 1000,
        whitelist: ["testnet_chat", gameState.board_game_contract, gameState.crash_game_contract],
    };
    return ret;
});
