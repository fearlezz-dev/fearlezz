import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

/// Wraps a widget so [still] can find and render it.
Widget lookable(Widget child) => RepaintBoundary(key: _boundary, child: child);

final GlobalKey _boundary = GlobalKey();

/// Renders the widget wrapped by [lookable] to a PNG in $FEARLEZZ_OUT (build/look when run
/// by hand) and lists it in proof.jsonl as a still, so Fearlezz compares it with the same
/// test run on the base commit.
Future<void> still(WidgetTester tester, String id) async {
  final out = Platform.environment['FEARLEZZ_OUT'] ?? 'build/look';
  final boundary =
      _boundary.currentContext!.findRenderObject()! as RenderRepaintBoundary;
  await tester.runAsync(() async {
    final image = await boundary.toImage(pixelRatio: 2);
    final png = await image.toByteData(format: ui.ImageByteFormat.png);
    image.dispose();
    Directory(out).createSync(recursive: true);
    File('$out/$id.png').writeAsBytesSync(png!.buffer.asUint8List());
    File('$out/proof.jsonl').writeAsStringSync(
      '${jsonEncode({'kind': 'still', 'file': '$id.png', 'id': id})}\n',
      mode: FileMode.append,
    );
  });
}
