# Vendored Website Libraries

KeyReed's marketing site ships a strict `Content-Security-Policy` (`script-src 'self'`), no build step, and zero external CDNs or trackers. Every third-party JavaScript library is vendored here with its exact licence and SHA-256 digest.

## Inventory

| File | Package & Version | Licence | SHA-256 |
| --- | --- | --- | --- |
| `gsap/gsap.min.js` | `gsap@3.15.1` | Standard "No Charge" GSAP License (`gsap/LICENSE.txt`) | `3d1bea374ab1b703cc5c4105358eec63c4465efdc573931dc116e34a0a190c1b` |
| `gsap/ScrollTrigger.min.js` | `gsap@3.15.1` | Standard "No Charge" GSAP License (`gsap/LICENSE.txt`) | `b5bb360661f478f0ff6ffa11500d8ce5e2b6aaf6f7288237cdca3aac9548035a` |
| `gsap/SplitText.min.js` | `gsap@3.15.1` | Standard "No Charge" GSAP License (`gsap/LICENSE.txt`) | `7b299a6b26b36fd5ce1c0712f31d6d25f98f9a9860778c2d3a78ce8f880f2edf` |
| `gsap/LICENSE.txt` | `gsap@3.15.1` | Standard "No Charge" GSAP License (`https://gsap.com/standard-license`) | `cea6fafdaabf157b29059795993cc5e5b378c800b7ea0ebd2f15f6da36283af3` |
| `lenis/lenis.min.js` | `lenis@1.3.27` | MIT License (`lenis/LICENSE`) | `80f53e77f1c13e379fc246d2b5e299188e20bcb2b2fe7ebb13e8e8c7563a6da7` |
| `lenis/lenis.min.js.map` | `lenis@1.3.27` | MIT License (`lenis/LICENSE`) | `f6d9c03617f7875884d5c07a6044661b21cfd522faf894263b106400d6287c33` |
| `lenis/LICENSE` | `lenis@1.3.27` | MIT License (`https://github.com/darkroomengineering/lenis`) | `7d1896b31f2eed9f286b05faca067c4e35b2f109ae7c4ed5105220e2d2820228` |

## Upstream Sources

- **GSAP (`gsap@3.15.1`)**: `https://registry.npmjs.org/gsap/-/gsap-3.15.1.tgz` — GreenSock / Webflow Standard "No Charge" License (`https://gsap.com/standard-license`, effective 2025-04-30, modified 2025-05-30). Permits reproducing, displaying, and implementing all GSAP Products (including `ScrollTrigger` and `SplitText`) on websites and web applications without charge, provided proprietary notices remain intact.
- **Lenis (`lenis@1.3.27`)**: `https://registry.npmjs.org/lenis/-/lenis-1.3.27.tgz` — MIT License (`Copyright (c) 2024 darkroom.engineering`).
