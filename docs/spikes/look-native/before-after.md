# Native looks: before and after

Spike, throwaway. What a Flutter app's `fearlezz.toml` looks like today, and with the
sketched pieces.

## Flutter, today

The project writes the look check by hand, and must keep `flutter-test` from recording too:

```toml
use = ["flutter"]

[check.flutter-test]
run = "flutter test --exclude-tags look"

[check.flutter-look]
run = "flutter test --tags look"
stage = "look"
inputs = ["lib/**", "test/**", "pubspec.yaml", "pubspec.lock", "dart_test.yaml", "assets/**"]
```

It also needs `dart_test.yaml` declaring the tag, and the helper (`flutter/still.dart`,
`flutter/flutter_test_config.dart`) copied into `test/`.

## Flutter, with the piece

```toml
use = ["flutter", "flutter-look"]
```

## macOS app, with the piece

```toml
use = ["macos-look"]

[check.macos-look]
inputs = ["App/**", "App.xcodeproj/**", "scripts/look.sh"]
```

`scripts/look.sh` builds the app, starts it, and records (see `macos-look.sh`):

```sh
look-record still --pid "$app" --out home.png --id home --crop-top 32
look-record film  --pid "$app" --out home.mp4 --id home-film --crop-top 32 --seconds 2
```
