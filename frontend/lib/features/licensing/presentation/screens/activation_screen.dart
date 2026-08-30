import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../core/theme/app_colors.dart';
import '../providers/license_providers.dart';

/// Pantalla de activación. Es el primer contacto del cliente con la aplicación,
/// así que su único trabajo es dejar clarísimos los dos pasos: copiar el ID del
/// equipo y pegar la licencia que recibe a cambio.
class ActivationScreen extends ConsumerStatefulWidget {
  const ActivationScreen({super.key});

  @override
  ConsumerState<ActivationScreen> createState() => _ActivationScreenState();
}

class _ActivationScreenState extends ConsumerState<ActivationScreen> {
  final _controller = TextEditingController();
  final _focusNode = FocusNode();
  bool _isLoading = false;
  bool _loadingCode = true;
  String? _hardwareCode;
  bool _hardwareFailed = false;

  @override
  void initState() {
    super.initState();
    _loadHardwareCode();
  }

  /// Calcula el ID del equipo. Si el hardware no responde —WMI bloqueado, un
  /// perfil sin permisos— hay que decirlo y ofrecer reintentar: un indicador de
  /// progreso eterno deja al cliente sin nada que hacer y sin saber por qué.
  Future<void> _loadHardwareCode() async {
    if (!_loadingCode) setState(() => _loadingCode = true);
    try {
      final fingerprint =
          await ref.read(licenseManagerProvider).getHardwareFingerprint();
      if (!mounted) return;
      setState(() {
        _hardwareCode = fingerprint;
        _hardwareFailed = false;
        _loadingCode = false;
      });
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _hardwareCode = null;
        _hardwareFailed = true;
        _loadingCode = false;
      });
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    _focusNode.dispose();
    super.dispose();
  }

  Future<void> _activate() async {
    final key = _controller.text.trim();
    if (key.isEmpty || _isLoading) return;

    setState(() => _isLoading = true);
    await ref.read(licenseProvider.notifier).activate(key);
    if (mounted) setState(() => _isLoading = false);
  }

  void _copyId() {
    final code = _hardwareCode;
    if (code == null) return;
    Clipboard.setData(ClipboardData(text: code));
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(
        const SnackBar(
          duration: Duration(seconds: 2),
          behavior: SnackBarBehavior.floating,
          width: 320,
          backgroundColor: AppColors.textPrimary,
          content: Text(
            'ID del dispositivo copiado al portapapeles',
            style: TextStyle(color: Colors.white, fontSize: 13),
          ),
        ),
      );
  }

  @override
  Widget build(BuildContext context) {
    final licenseState = ref.watch(licenseProvider);
    final screenWidth = MediaQuery.sizeOf(context).width;
    final cardWidth = screenWidth < 520 ? screenWidth - 48 : 460.0;

    return Scaffold(
      backgroundColor: AppColors.background,
      body: Stack(
        children: [
          Center(
            child: SingleChildScrollView(
              padding: const EdgeInsets.symmetric(vertical: 32, horizontal: 24),
              child: SizedBox(
                width: cardWidth,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    _buildHeader(),
                    const SizedBox(height: 28),
                    _buildDeviceIdBlock(),
                    const SizedBox(height: 20),
                    if (licenseState.error != null) ...[
                      _buildErrorBlock(licenseState.error!),
                      const SizedBox(height: 16),
                    ],
                    _buildKeyField(),
                    const SizedBox(height: 16),
                    _buildActivateButton(),
                    const SizedBox(height: 20),
                    const Text(
                      'La licencia se verifica en tu equipo, sin conexión a '
                      'internet y sin enviar ningún dato.',
                      textAlign: TextAlign.center,
                      style: TextStyle(
                        fontSize: 11,
                        height: 1.5,
                        color: AppColors.textDisabled,
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
          if (_isLoading) _buildLoadingOverlay(),
        ],
      ),
    );
  }

  Widget _buildHeader() {
    return Column(
      children: [
        ClipRRect(
          borderRadius: BorderRadius.circular(16),
          child: Image.asset(
            'assets/images/logo.png',
            width: 64,
            height: 64,
            fit: BoxFit.cover,
          ),
        ),
        const SizedBox(height: 16),
        const Text(
          'BDJ Studio Search Pro',
          textAlign: TextAlign.center,
          style: TextStyle(
            fontSize: 20,
            fontWeight: FontWeight.w700,
            color: AppColors.textPrimary,
            letterSpacing: -0.2,
          ),
        ),
        const SizedBox(height: 6),
        const Text(
          'Activa tu licencia para empezar a buscar',
          textAlign: TextAlign.center,
          style: TextStyle(fontSize: 13, color: AppColors.textSecondary),
        ),
      ],
    );
  }

  Widget _buildDeviceIdBlock() {
    final code = _hardwareCode;
    return Container(
      padding: const EdgeInsets.fromLTRB(20, 18, 20, 18),
      decoration: BoxDecoration(
        color: AppColors.surface,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: AppColors.border),
      ),
      child: Column(
        children: [
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              const Text(
                '1',
                style: TextStyle(
                  fontSize: 11,
                  fontWeight: FontWeight.w700,
                  color: AppColors.primary,
                ),
              ),
              const SizedBox(width: 6),
              Text(
                'ID DEL DISPOSITIVO',
                style: TextStyle(
                  fontSize: 10.5,
                  fontWeight: FontWeight.w700,
                  letterSpacing: 0.8,
                  color: AppColors.textSecondary.withValues(alpha: 0.9),
                ),
              ),
            ],
          ),
          const SizedBox(height: 12),
          if (_loadingCode)
            const SizedBox(
              height: 24,
              width: 24,
              child: CircularProgressIndicator(
                strokeWidth: 2,
                color: AppColors.primary,
              ),
            )
          else if (_hardwareFailed)
            const Text(
              'No se pudo leer el identificador de este equipo.',
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: 12.5,
                height: 1.4,
                color: AppColors.error,
                fontWeight: FontWeight.w500,
              ),
            )
          else
            FittedBox(
              fit: BoxFit.scaleDown,
              child: SelectableText(
                code ?? '',
                style: const TextStyle(
                  fontSize: 19,
                  color: AppColors.primary,
                  fontFamily: 'monospace',
                  fontWeight: FontWeight.w700,
                  letterSpacing: 1.4,
                ),
              ),
            ),
          const SizedBox(height: 14),
          if (_hardwareFailed)
            OutlinedButton.icon(
              onPressed: _loadHardwareCode,
              icon: const Icon(Icons.refresh_rounded, size: 15),
              label: const Text('Reintentar'),
              style: _idButtonStyle,
            )
          else
            OutlinedButton.icon(
              onPressed: code == null ? null : _copyId,
              icon: const Icon(Icons.copy_rounded, size: 15),
              label: const Text('Copiar ID'),
              style: _idButtonStyle,
            ),
          const SizedBox(height: 10),
          Text(
            _hardwareFailed
                ? 'Sin este código no se puede emitir tu licencia.'
                : 'Envía este código para recibir tu licencia.',
            textAlign: TextAlign.center,
            style: const TextStyle(
              fontSize: 11.5,
              color: AppColors.textSecondary,
            ),
          ),
        ],
      ),
    );
  }

  ButtonStyle get _idButtonStyle => OutlinedButton.styleFrom(
        foregroundColor: AppColors.primary,
        side: const BorderSide(color: AppColors.borderStrong),
        backgroundColor: AppColors.background,
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 10),
        textStyle: const TextStyle(
          fontSize: 12.5,
          fontWeight: FontWeight.w600,
        ),
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(8),
        ),
      );

  Widget _buildErrorBlock(String message) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      decoration: BoxDecoration(
        color: AppColors.error.withValues(alpha: 0.07),
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: AppColors.error.withValues(alpha: 0.28)),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Icon(Icons.error_outline_rounded,
              size: 17, color: AppColors.error),
          const SizedBox(width: 10),
          Expanded(
            child: Text(
              message,
              style: const TextStyle(
                color: AppColors.error,
                fontSize: 12,
                height: 1.45,
                fontWeight: FontWeight.w500,
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildKeyField() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            const Text(
              '2',
              style: TextStyle(
                fontSize: 11,
                fontWeight: FontWeight.w700,
                color: AppColors.primary,
              ),
            ),
            const SizedBox(width: 6),
            Text(
              'LLAVE DE LICENCIA',
              style: TextStyle(
                fontSize: 10.5,
                fontWeight: FontWeight.w700,
                letterSpacing: 0.8,
                color: AppColors.textSecondary.withValues(alpha: 0.9),
              ),
            ),
          ],
        ),
        const SizedBox(height: 8),
        TextField(
          controller: _controller,
          focusNode: _focusNode,
          autofocus: true,
          maxLines: 3,
          minLines: 2,
          style: const TextStyle(
            color: AppColors.textPrimary,
            fontSize: 12,
            fontFamily: 'monospace',
            height: 1.5,
          ),
          decoration: InputDecoration(
            hintText: 'Pega aquí tu licencia SPP3',
            hintStyle: const TextStyle(
              color: AppColors.textDisabled,
              fontFamily: 'monospace',
              fontSize: 12,
            ),
            helperText: 'Formato: SPP3.<contenido>.<certificado>.<firma>',
            helperStyle: const TextStyle(
              fontSize: 10.5,
              color: AppColors.textDisabled,
            ),
            filled: true,
            fillColor: AppColors.surface,
            contentPadding:
                const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
            border: OutlineInputBorder(
              borderRadius: BorderRadius.circular(10),
              borderSide: const BorderSide(color: AppColors.border),
            ),
            enabledBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(10),
              borderSide: const BorderSide(color: AppColors.border),
            ),
            focusedBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(10),
              borderSide: const BorderSide(color: AppColors.primary, width: 1.6),
            ),
          ),
          onSubmitted: (_) => _activate(),
        ),
      ],
    );
  }

  Widget _buildActivateButton() {
    return SizedBox(
      height: 46,
      child: ElevatedButton(
        onPressed: _isLoading ? null : _activate,
        style: ElevatedButton.styleFrom(
          backgroundColor: AppColors.primary,
          foregroundColor: Colors.white,
          disabledBackgroundColor: AppColors.borderStrong,
          elevation: 0,
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(10),
          ),
        ),
        child: const Text(
          'Activar aplicación',
          style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600),
        ),
      ),
    );
  }

  Widget _buildLoadingOverlay() {
    return Positioned.fill(
      child: ColoredBox(
        color: AppColors.background.withValues(alpha: 0.72),
        child: const Center(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              SizedBox(
                height: 26,
                width: 26,
                child: CircularProgressIndicator(
                  strokeWidth: 2.4,
                  color: AppColors.primary,
                ),
              ),
              SizedBox(height: 14),
              Text(
                'Verificando licencia...',
                style: TextStyle(
                  fontSize: 13,
                  color: AppColors.textSecondary,
                  fontWeight: FontWeight.w500,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
