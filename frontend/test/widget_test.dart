import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:bdj_studio_search_pro/main.dart';

void main() {
  testWidgets('SearchProApp loads and displays search UI elements', (WidgetTester tester) async {
    await tester.pumpWidget(const ProviderScope(child: SearchProApp()));
    expect(find.text('BDJ Studio Search Pro'), findsOneWidget);
    expect(find.text('Todos'), findsOneWidget);
    expect(find.text('Audio'), findsOneWidget);
    expect(find.textContaining('Nombre'), findsOneWidget);
  });
}
