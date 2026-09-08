import 'package:flutter_test/flutter_test.dart';
import 'package:bdj_studio_search_pro/features/fileops/providers/file_ops_provider.dart';

void main() {
  group('FileOps State & Clipboard Logic', () {
    test('ClipboardContents isEmpty and isNotEmpty behave as expected', () {
      const empty = ClipboardContents();
      expect(empty.isEmpty, true);
      expect(empty.isNotEmpty, false);
      expect(empty.isCut, false);
      expect(empty.paths, isEmpty);

      const filledCopy = ClipboardContents(paths: ['C:\\a.mp3', 'C:\\b.wav'], isCut: false);
      expect(filledCopy.isEmpty, false);
      expect(filledCopy.isNotEmpty, true);
      expect(filledCopy.isCut, false);
      expect(filledCopy.paths.length, 2);

      const filledCut = ClipboardContents(paths: ['C:\\a.mp3'], isCut: true);
      expect(filledCut.isEmpty, false);
      expect(filledCut.isNotEmpty, true);
      expect(filledCut.isCut, true);
    });

    test('FileOpsState initial values and copyWith', () {
      const initial = FileOpsState();
      expect(initial.operations, isEmpty);
      expect(initial.clipboard.isEmpty, true);
      expect(initial.canUndo, false);
      expect(initial.canRedo, false);
      expect(initial.hasActiveWork, false);
      expect(initial.pendingConflict, isNull);

      final modified = initial.copyWith(
        canUndo: true,
        canRedo: true,
        clipboard: const ClipboardContents(paths: ['C:\\test.mp3'], isCut: true),
      );
      expect(modified.canUndo, true);
      expect(modified.canRedo, true);
      expect(modified.clipboard.paths, ['C:\\test.mp3']);
      expect(modified.clipboard.isCut, true);
    });

    test('ConflictPolicy and ConflictDecision constants match contract', () {
      expect(ConflictPolicy.ask, 0);
      expect(ConflictPolicy.keepBoth, 1);
      expect(ConflictPolicy.skip, 2);
      expect(ConflictPolicy.overwrite, 3);

      expect(ConflictDecision.keepBoth, 0);
      expect(ConflictDecision.skip, 1);
      expect(ConflictDecision.overwrite, 2);
      expect(ConflictDecision.cancel, 3);
    });

    test('OpState lifecycle constants are mapped correctly', () {
      expect(OpState.planning, 0);
      expect(OpState.running, 1);
      expect(OpState.waitingConflict, 2);
      expect(OpState.cancelling, 3);
      expect(OpState.done, 4);
      expect(OpState.cancelled, 5);
      expect(OpState.failed, 6);
    });
  });
}
