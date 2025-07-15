# changelog

---

## [0.3.0](https://git.musicaloft.com/municorn/munibot/compare/6a976b1d51ea57dfa29e7a3a8e264b5a056c0038..0.3.0) - 2025-07-15

#### bug fixes

- **(autodelete)** fix text spacing in message -
  ([dcabd89](https://git.musicaloft.com/municorn/munibot/commit/dcabd899eed670b651a62d2b87acef82d85f868b)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(logging)** fix a few issues with guild updates -
  ([dcbcf27](https://git.musicaloft.com/municorn/munibot/commit/dcbcf27737a0b8358287ea7396d9c169f6b0813e)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(logging)** fix spacing in channel delete handler -
  ([c2a55af](https://git.musicaloft.com/municorn/munibot/commit/c2a55aff94992504e7eaad9ed415da9c71305ebc)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(logging)** fix channel delete message typo -
  ([83b12c6](https://git.musicaloft.com/municorn/munibot/commit/83b12c69d10db3cb0ff003246c435cddfe58c127)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(vc_greeter)** push nickname safely to prevent everyone pings -
  ([573ea23](https://git.musicaloft.com/municorn/munibot/commit/573ea23c0303d322c78ad0b5ccbc2795fc57f0c7)) -
  [@municorn](https://git.musicaloft.com/municorn)
- duplicate import -
  ([e5f065f](https://git.musicaloft.com/municorn/munibot/commit/e5f065f0ddb092a8ac2a83d752150194baab321c)) -
  [@municorn](https://git.musicaloft.com/municorn)

#### build system

- update crate features -
  ([b2fec4d](https://git.musicaloft.com/municorn/munibot/commit/b2fec4d92b364b694e4a2bb2c3c9e501073d6b41)) -
  [@municorn](https://git.musicaloft.com/municorn)
- pin libressl to libressl_4_0 -
  ([af1857b](https://git.musicaloft.com/municorn/munibot/commit/af1857beda619b5ac07690d1c7b596982f40a5ab)) -
  [@municorn](https://git.musicaloft.com/municorn)
- update Rust edition from 2021 to 2024 -
  ([6ec4144](https://git.musicaloft.com/municorn/munibot/commit/6ec41442001f1bbf53d9103388bf925488aace4e)) -
  [@municorn](https://git.musicaloft.com/municorn)
- correct stdenv adapter function call syntax in flake.nix -
  ([18982d1](https://git.musicaloft.com/municorn/munibot/commit/18982d1df55fe387cdb6e31b9a190992f407685a)) -
  [@municorn](https://git.musicaloft.com/municorn)

#### new features

- **(admin)** make admin responses ephemeral -
  ([4a03af2](https://git.musicaloft.com/municorn/munibot/commit/4a03af22176f909f60942cc937814a4000be1721)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(autodelete)** use LoggingHandler's ignore feature for less annoying logs -
  ([64f2019](https://git.musicaloft.com/municorn/munibot/commit/64f20195084810cc30cb760bd624a505c0486701)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(bot_affection)** remove more affectionate commands -
  ([f06bc2e](https://git.musicaloft.com/municorn/munibot/commit/f06bc2e13c2d85436075ec7f32d230e74df829b2)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(logging)** allow message ids to be ignored when logging -
  ([7e8823d](https://git.musicaloft.com/municorn/munibot/commit/7e8823d5844134bc42b2b7e78210f3069ac7af81)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(logging)** fix some formatting for guild updates -
  ([adcc401](https://git.musicaloft.com/municorn/munibot/commit/adcc4018c64232807e13923cb64283d0335ce4a9)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(logging)** implement logging for guild updates -
  ([c2f5580](https://git.musicaloft.com/municorn/munibot/commit/c2f5580aa0a15fe1a4a83db756cb9106d37820e4)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(logging)** don't log `position` changes in role updates -
  ([86cd567](https://git.musicaloft.com/municorn/munibot/commit/86cd5677af9443086770097b68b79cf6530ff3e1)) -
  [@municorn](https://git.musicaloft.com/municorn)
- **(simple)** make tone indicator guide ephemeral -
  ([5d458ba](https://git.musicaloft.com/municorn/munibot/commit/5d458baaee8c6dc5f3e8d64f6ee0f8cc03466804)) -
  [@municorn](https://git.musicaloft.com/municorn)
- add treefmt-nix for unified code formatting -
  ([34c062d](https://git.musicaloft.com/municorn/munibot/commit/34c062d40cf8d327bd8ffe7a141b16da2b9b1fe3)) -
  [@municorn](https://git.musicaloft.com/municorn)
- rename to munibot, underscore removed -
  ([48eb65f](https://git.musicaloft.com/municorn/munibot/commit/48eb65faf473875eb7924fdf103870e59f99fb10)) -
  [@municorn](https://git.musicaloft.com/municorn)
- update LICENSE and README -
  ([4033b5e](https://git.musicaloft.com/municorn/munibot/commit/4033b5ed212afad403d1befbbde5b39164e360c6)) -
  [@municorn](https://git.musicaloft.com/municorn)
- create FUNDING.yml -
  ([6a976b1](https://git.musicaloft.com/municorn/munibot/commit/6a976b1d51ea57dfa29e7a3a8e264b5a056c0038)) -
  [@municorn](https://git.musicaloft.com/municorn)

#### performance

- box thicc `MuniBotError`s -
  ([453b68a](https://git.musicaloft.com/municorn/munibot/commit/453b68a6a199a5a0c2b464afb572d0b56920d001)) -
  [@municorn](https://git.musicaloft.com/municorn)

#### refactors

- **(greeting)** remove special handling for Linokii -
  ([09ea687](https://git.musicaloft.com/municorn/munibot/commit/09ea6870fb64ef4f1ecd038c9f0e492a918d93e5)) -
  [@municorn](https://git.musicaloft.com/municorn)
- use inline string formatting in Rust format! macros -
  ([636f355](https://git.musicaloft.com/municorn/munibot/commit/636f35552313432ad2469e7d5836e6f5cdfca702)) -
  [@municorn](https://git.musicaloft.com/municorn)
- use let-chains to simplify nested if-let expressions -
  ([f1d4741](https://git.musicaloft.com/municorn/munibot/commit/f1d47413e43beaaddf94f3854b742049f7e9a33b)) -
  [@municorn](https://git.musicaloft.com/municorn)

---

changelog generated by [cocogitto](https://github.com/cocogitto/cocogitto)
