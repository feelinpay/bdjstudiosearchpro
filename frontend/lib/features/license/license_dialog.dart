import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../../core/ffi/api.dart' as ffi;
import '../../core/theme/app_colors.dart';

class LicenseDialog extends StatefulWidget {
  const LicenseDialog({super.key});

  static Future<void> show(BuildContext context) {
    return showDialog(
      context: context,
      builder: (context) => const LicenseDialog(),
    );
  }

  @override
  State<LicenseDialog> createState() => _LicenseDialogState();
}

class _LicenseDialogState extends State<LicenseDialog> {
  final TextEditingController _keyController = TextEditingController();
  ffi.LicenseInfoFfi? _info;
  bool _isLoading = true;
  String? _errorMessage;
  String? _successMessage;

  @override
  void initState() {
    super.initState();
    _loadStatus();
  }

  @override
  void dispose() {
    _keyController.dispose();
    super.dispose();
  }

  Future<void> _loadStatus() async {
    setState(() => _isLoading = true);
    try {
      final info = await ffi.getLicenseStatus();
      setState(() {
        _info = info;
        _isLoading = false;
      });
    } catch (e) {
      setState(() => _isLoading = false);
    }
  }

  Future<void> _activate() async {
    final key = _keyController.text.trim();
    if (key.isEmpty) return;

    setState(() {
      _isLoading = true;
      _errorMessage = null;
      _successMessage = null;
    });

    try {
      final updated = await ffi.activateProductKey(key: key);
      setState(() {
        _info = updated;
        _isLoading = false;
        _successMessage = '¡Licencia activada con éxito para BDJ Studio Search Pro!';
      });
    } catch (e) {
      setState(() {
        _isLoading = false;
        _errorMessage = e.toString().replaceAll('Exception: ', '');
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Dialog(
      backgroundColor: AppColors.background,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(8),
        side: const BorderSide(color: AppColors.border),
      ),
      child: Container(
        width: 480,
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                const Icon(Icons.verified_user_rounded, color: AppColors.primary, size: 24),
                const SizedBox(width: 10),
                const Text(
                  'Licencia y Activación',
                  style: TextStyle(
                    fontSize: 16,
                    fontWeight: FontWeight.w600,
                    color: AppColors.textPrimary,
                  ),
                ),
                const Spacer(),
                IconButton(
                  icon: const Icon(Icons.close, size: 18, color: AppColors.textSecondary),
                  onPressed: () => Navigator.of(context).pop(),
                ),
              ],
            ),
            const SizedBox(height: 16),

            if (_isLoading)
              const Center(
                child: Padding(
                  padding: EdgeInsets.all(24),
                  child: CircularProgressIndicator(strokeWidth: 2),
                ),
              )
            else ...[
              // HWID Information
              Container(
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: AppColors.surface,
                  borderRadius: BorderRadius.circular(6),
                  border: Border.all(color: AppColors.border),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        const Text(
                          'Identificador de Hardware (HWID V2):',
                          style: TextStyle(fontSize: 11, color: AppColors.textSecondary),
                        ),
                        InkWell(
                          onTap: () {
                            if (_info != null) {
                              Clipboard.setData(ClipboardData(text: _info!.hwid));
                              ScaffoldMessenger.of(context).showSnackBar(
                                const SnackBar(
                                  content: Text('HWID copiado al portapapeles'),
                                  duration: Duration(milliseconds: 1000),
                                ),
                              );
                            }
                          },
                          child: const Row(
                            children: [
                              Icon(Icons.copy_rounded, size: 12, color: AppColors.primary),
                              SizedBox(width: 4),
                              Text(
                                'Copiar',
                                style: TextStyle(
                                  fontSize: 11,
                                  fontWeight: FontWeight.w500,
                                  color: AppColors.primary,
                                ),
                              ),
                            ],
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 4),
                    Text(
                      _info?.hwid ?? 'Cargando...',
                      style: const TextStyle(
                        fontSize: 13,
                        fontFamily: 'monospace',
                        fontWeight: FontWeight.w600,
                        color: AppColors.textPrimary,
                      ),
                    ),
                    const Divider(height: 16, color: AppColors.border),
                    Row(
                      children: [
                        const Text('Estado: ', style: TextStyle(fontSize: 12, color: AppColors.textSecondary)),
                        Text(
                          _info?.isTrial == true
                              ? 'Prueba Gratuita (${_info?.trialDaysLeft} días restantes)'
                              : (_info?.isValid == true ? 'Licencia Comercial Activada' : 'Licencia Expirada'),
                          style: TextStyle(
                            fontSize: 12,
                            fontWeight: FontWeight.w600,
                            color: _info?.isValid == true ? AppColors.success : AppColors.error,
                          ),
                        ),
                      ],
                    ),
                  ],
                ),
              ),

              const SizedBox(height: 20),
              const Text(
                'Introduce tu clave de producto:',
                style: TextStyle(fontSize: 12, fontWeight: FontWeight.w500, color: AppColors.textPrimary),
              ),
              const SizedBox(height: 8),

              TextField(
                controller: _keyController,
                style: const TextStyle(
                  fontSize: 13,
                  fontFamily: 'monospace',
                  letterSpacing: 1.2,
                  color: AppColors.textPrimary,
                ),
                decoration: InputDecoration(
                  hintText: 'BDJ6-XXXX-XXXX-XXXX-XXXX',
                  hintStyle: const TextStyle(color: AppColors.textDisabled, letterSpacing: 1.0),
                  filled: true,
                  fillColor: AppColors.surface,
                  contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
                  border: OutlineInputBorder(
                    borderRadius: BorderRadius.circular(6),
                    borderSide: const BorderSide(color: AppColors.border),
                  ),
                  enabledBorder: OutlineInputBorder(
                    borderRadius: BorderRadius.circular(6),
                    borderSide: const BorderSide(color: AppColors.border),
                  ),
                ),
              ),

              if (_errorMessage != null) ...[
                const SizedBox(height: 10),
                Text(
                  _errorMessage!,
                  style: const TextStyle(fontSize: 12, color: AppColors.error),
                ),
              ],

              if (_successMessage != null) ...[
                const SizedBox(height: 10),
                Text(
                  _successMessage!,
                  style: const TextStyle(fontSize: 12, color: AppColors.success, fontWeight: FontWeight.w500),
                ),
              ],

              const SizedBox(height: 20),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  TextButton(
                    onPressed: () => Navigator.of(context).pop(),
                    child: const Text('Cerrar', style: TextStyle(color: AppColors.textSecondary)),
                  ),
                  const SizedBox(width: 8),
                  ElevatedButton(
                    onPressed: _activate,
                    style: ElevatedButton.styleFrom(
                      backgroundColor: AppColors.primary,
                      foregroundColor: Colors.white,
                      padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 10),
                      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
                    ),
                    child: const Text('Activar Licencia', style: TextStyle(fontSize: 13)),
                  ),
                ],
              ),
            ],
          ],
        ),
      ),
    );
  }
}
