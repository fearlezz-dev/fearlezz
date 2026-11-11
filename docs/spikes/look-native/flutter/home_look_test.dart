@Tags(['look'])
library;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:look_app/main.dart';

import 'still.dart';

void main() {
  setUp(() {
    final view =
        TestWidgetsFlutterBinding.instance.platformDispatcher.views.first;
    view.physicalSize = const Size(800, 600);
    view.devicePixelRatio = 1;
  });

  testWidgets('home', (tester) async {
    await tester.pumpWidget(lookable(const MyApp()));
    await still(tester, 'home');
  });

  testWidgets('home after three taps', (tester) async {
    await tester.pumpWidget(lookable(const MyApp()));
    for (var i = 0; i < 3; i++) {
      await tester.tap(find.byIcon(Icons.add));
    }
    await tester.pumpAndSettle();
    await still(tester, 'home-tapped');
  });
}
