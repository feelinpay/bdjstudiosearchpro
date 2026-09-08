import 'dart:io';
import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

final autostartProvider = StateNotifierProvider<AutostartNotifier, bool>((ref) {
  return AutostartNotifier();
});

class AutostartNotifier extends StateNotifier<bool> {
  AutostartNotifier() : super(false) {
    _init();
  }

  Future<void> _init() async {
    state = await AutostartService.isEnabled();
  }

  Future<void> toggle() async {
    final next = !state;
    final ok = await AutostartService.setEnabled(next);
    if (ok) {
      state = next;
    }
  }
}

class AutostartService {
  static const String appName = 'BDJ Studio Search Pro';
  static const String macPlistId = 'com.bdjstudio.searchpro';

  /// Comprueba si la aplicación está configurada para iniciar con el sistema.
  static Future<bool> isEnabled() async {
    try {
      if (Platform.isWindows) {
        final result = await Process.run('reg', [
          'query',
          r'HKCU\Software\Microsoft\Windows\CurrentVersion\Run',
          '/v',
          appName,
        ]);
        return result.exitCode == 0;
      } else if (Platform.isMacOS) {
        final home = Platform.environment['HOME'] ?? '';
        if (home.isEmpty) return false;
        final file = File('$home/Library/LaunchAgents/$macPlistId.plist');
        return file.existsSync();
      }
    } catch (e) {
      debugPrint('Autostart check note: $e');
    }
    return false;
  }

  /// Activa o desactiva el inicio automático con el sistema operativo en segundo plano.
  static Future<bool> setEnabled(bool enable) async {
    try {
      if (Platform.isWindows) {
        if (enable) {
          final exePath = Platform.resolvedExecutable;
          final result = await Process.run('reg', [
            'add',
            r'HKCU\Software\Microsoft\Windows\CurrentVersion\Run',
            '/v',
            appName,
            '/t',
            'REG_SZ',
            '/d',
            '"$exePath" --startup',
            '/f',
          ]);
          return result.exitCode == 0;
        } else {
          final result = await Process.run('reg', [
            'delete',
            r'HKCU\Software\Microsoft\Windows\CurrentVersion\Run',
            '/v',
            appName,
            '/f',
          ]);
          return result.exitCode == 0;
        }
      } else if (Platform.isMacOS) {
        final home = Platform.environment['HOME'] ?? '';
        if (home.isEmpty) return false;
        final dir = Directory('$home/Library/LaunchAgents');
        if (!dir.existsSync()) {
          dir.createSync(recursive: true);
        }
        final file = File('$home/Library/LaunchAgents/$macPlistId.plist');
        if (enable) {
          final exePath = Platform.resolvedExecutable;
          final plistContent = '''<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$macPlistId</string>
    <key>ProgramArguments</key>
    <array>
        <string>$exePath</string>
        <string>--startup</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>
''';
          await file.writeAsString(plistContent);
          return true;
        } else {
          if (file.existsSync()) {
            await file.delete();
          }
          return true;
        }
      }
    } catch (e) {
      debugPrint('Autostart toggle error: $e');
    }
    return false;
  }
}
