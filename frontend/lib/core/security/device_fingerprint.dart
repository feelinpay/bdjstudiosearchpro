import 'dart:convert';
import 'dart:io';

import 'package:bdj_license_core/bdj_license_core.dart';
import 'package:device_info_plus/device_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'security_port.dart';

/// Genera la huella de dispositivo (HWID) siguiendo el estándar bdj_license_core V2,
/// basada en identificadores de hardware físicos inmutables que persisten ante formateos.
///
/// Consistencia entre apps: el algoritmo y las fuentes de hardware son idénticos en
/// todas las apps BDJ Studio, por lo que en la misma máquina el ID es el mismo que
/// muestran Sample Pad, Stems Music y el resto de la suite.
///
/// ESTE ARCHIVO ES COMÚN A TODA LA SUITE. No lo modifiques aquí sin aplicar el mismo
/// cambio en las demás aplicaciones: cualquier divergencia rompe la identidad del
/// dispositivo y deja licencias emitidas sin poder activarse.
///
/// Persistencia y reutilización:
///  - El primer arranque calcula el HWID desde el hardware y lo guarda junto a una
///    "firma de estabilidad" (hash de los componentes NO volátiles).
///  - En arranques posteriores se reutiliza el HWID guardado SIEMPRE que la firma de
///    estabilidad siga coincidiendo (mismo hardware físico), aunque el sistema se haya
///    formateado o el identificador volátil (ANDROID_ID / identifierForVendor) cambie.
class DeviceFingerprint {
  DeviceFingerprint({this.readPersisted, this.writePersisted});

  /// Clave compartida para el registro persistente (HWID + firma de estabilidad).
  static const String persistedKey = 'bdj.hwid.v2';

  final Future<String?> Function()? readPersisted;
  final Future<void> Function(String value)? writePersisted;

  final DeviceInfoPlugin _deviceInfo = DeviceInfoPlugin();
  static String? _cachedFingerprint;

  /// Fábrica que conecta la persistencia al almacenamiento seguro de la app.
  static DeviceFingerprint withPersistentStorage(SecurityPort storage) {
    return DeviceFingerprint(
      readPersisted: () => _readPersistedValue(storage),
      writePersisted: (value) => _writePersistedValue(storage, value),
    );
  }

  static Future<String?> _readPersistedValue(SecurityPort storage) async {
    final secure = (await storage.readSecure(persistedKey)).getOrElse(() => null);
    if (secure != null && secure.isNotEmpty) return secure;
    if (Platform.isAndroid) {
      try {
        final prefs = await SharedPreferences.getInstance();
        return prefs.getString(persistedKey);
      } catch (_) {}
    }
    return null;
  }

  static Future<void> _writePersistedValue(
      SecurityPort storage, String value) async {
    await storage.storeSecure(persistedKey, value);
    if (Platform.isAndroid) {
      try {
        final prefs = await SharedPreferences.getInstance();
        await prefs.setString(persistedKey, value);
      } catch (_) {}
    }
  }

  Future<String> generate() async {
    if (_cachedFingerprint != null) return _cachedFingerprint!;
    final result = await generateResult();
    _cachedFingerprint = await _resolvePersisted(result);
    return _cachedFingerprint!;
  }

  Future<String> _resolvePersisted(HwidResult result) async {
    if (readPersisted == null || writePersisted == null) {
      return result.visibleHwid;
    }
    try {
      final stored = await readPersisted!();
      if (stored != null && stored.isNotEmpty) {
        final record = jsonDecode(stored);
        if (record is Map<String, dynamic>) {
          final storedFp = record['f'];
          final storedSig = record['s'];
          if (storedFp is String &&
              storedFp.isNotEmpty &&
              storedSig == result.stabilitySignature) {
            return storedFp;
          }
        }
      }
    } catch (_) {}

    try {
      await writePersisted!(
        jsonEncode(<String, String>{
          'f': result.visibleHwid,
          's': result.stabilitySignature,
        }),
      );
    } catch (_) {}
    return result.visibleHwid;
  }

  Future<HwidResult> generateResult() async {
    if (Platform.isAndroid) {
      final android = await _deviceInfo.androidInfo;
      return HwidEngine.canonicalize(
        platform: 'android',
        components: {
          'id': android.id,
          'brand': android.brand,
          'device': android.device,
          'hardware': android.hardware,
          'board': android.board,
          'model': android.model,
        },
      );
    } else if (Platform.isIOS) {
      final ios = await _deviceInfo.iosInfo;
      return HwidEngine.canonicalize(
        platform: 'ios',
        components: {
          'id': ios.identifierForVendor,
        },
      );
    } else if (Platform.isWindows) {
      // Asíncrono (nunca runSync): WMI vía PowerShell puede tardar varios
      // segundos en PCs lentas y síncrono congelaría el isolate de UI.
      final hwIds = await _getWindowsHardwareIds();
      if (hwIds.isNotEmpty && hwIds.containsKey('smbiosUuid')) {
        return HwidEngine.canonicalize(
          platform: 'windows',
          components: hwIds,
        );
      }

      // Fallback a machineGuid si WMI no está accesible
      final windows = await _deviceInfo.windowsInfo;
      return HwidEngine.canonicalize(
        platform: 'windows',
        components: {
          'deviceId': windows.deviceId,
        },
      );
    } else if (Platform.isMacOS) {
      final macos = await _deviceInfo.macOsInfo;
      return HwidEngine.canonicalize(
        platform: 'macos',
        components: {
          'systemGUID': macos.systemGUID,
        },
      );
    } else if (Platform.isLinux) {
      final linux = await _deviceInfo.linuxInfo;
      String? dmiUuid;
      try {
        final dmiFile = File('/sys/class/dmi/id/product_uuid');
        if (dmiFile.existsSync()) {
          dmiUuid = dmiFile.readAsStringSync().trim();
        }
      } catch (_) {}

      String? machineId = linux.machineId;
      if (machineId == null || machineId.isEmpty) {
        try {
          final midFile = File('/etc/machine-id');
          if (midFile.existsSync()) {
            machineId = midFile.readAsStringSync().trim();
          }
        } catch (_) {}
      }

      return HwidEngine.canonicalize(
        platform: 'linux',
        components: {
          'machineId': machineId,
          'productUuid': dmiUuid,
        },
      );
    }

    throw const DeviceFingerprintFailure(
      'Plataforma desconocida o no compatible (unknown_platform).',
    );
  }

  Future<Map<String, String>> _getWindowsHardwareIds() async {
    try {
      final result = await Process.run(
        'powershell',
        [
          '-NoProfile',
          '-NonInteractive',
          '-Command',
          r'$p = Get-CimInstance Win32_ComputerSystemProduct; $c = Get-CimInstance Win32_Processor; $b = Get-CimInstance Win32_BaseBoard; "$($p.UUID)`n$($c.ProcessorId)`n$($b.SerialNumber)"',
        ],
      );

      if (result.exitCode == 0) {
        final lines = (result.stdout as String)
            .split(RegExp(r'\r?\n'))
            .map((l) => l.trim())
            .where((l) => l.isNotEmpty)
            .toList();

        if (lines.isNotEmpty) {
          final uuid = lines[0];
          final cpu = lines.length > 1 ? lines[1] : '';
          final board = lines.length > 2 ? lines[2] : '';

          return {
            if (uuid.isNotEmpty && uuid.toLowerCase() != 'null')
              'smbiosUuid': uuid,
            if (cpu.isNotEmpty && cpu.toLowerCase() != 'null') 'cpuId': cpu,
            if (board.isNotEmpty && board.toLowerCase() != 'null')
              'baseboardSerial': board,
          };
        }
      }
    } catch (_) {}
    return {};
  }
}
