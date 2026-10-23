# Nikon Z 5 RAW colour regression: DSC_4814.NEF

Status: experimental; do not merge without real-NEF acceptance.

## Reproduction supplied on 2026-10-23

- Camera: Nikon Z 5; image: DSC_4814.NEF (6016x4016 displayed).
- FIT: camera JPEG looked natural, displayed in 42 ms in the user trace.
- Native 1:1: first implementation showed severe green/yellow cast in faces, clothing and foliage.
- Observed native stage times: rawler demosaic 1024 ms, camera-look fit 134 ms,
  full source decode ~1170 ms, full viewer display ~1305 ms.
- Embedded JPEG has native 6016x4016 pixels and no embedded ICC; PIC assumes sRGB.
- Rawler default RAW development is known to differ from manufacturer camera JPEG rendering.

## Root-cause evidence

The per-photo colour-matrix and global luminance-quantile calibration was insufficient.
Independent sensor development with LibRaw yielded natural skin colour on the same source
NEF, and a reproduction of PIC's global matrix + luminance mapping over that output
introduced a similar cyan/green cast. This implicates the *calibration design*, not
corruption of the original image or GTK presentation. In this photograph, foliage
dominates most pixel samples, while important skin and neutral-clothing patches are
poorly reproduced by a single global linear transform.

## Emergency quality gate

The experimental branch now checks neutral and skin colour shifts on sparse samples
before allocating the 24MP sensor-rendered output. On a rejected fit, the viewer
returns Nikon's full-resolution JPEG as a **temporary quality fallback**. Pixels
are not blended. This fallback is JPEG-derived and **must not be represented as
a true sensor RAW development**. The original NEF remains untouched. Missing JPEG
or decode failures still return rawler's sensor development.

Trace: `PIC_DECODE stage=nef_camera_render_match result=quality_rejected fallback=nikon_full_resolution_jpeg`.

## Acceptance before merge

1. Run the camera-look quality gate on DSC_4814.NEF with PICASA_TRACE=1.
   Verify whether the model is accepted/rejected; confirm there is no green 1:1 cast.
2. Prove that a real sensor-derived display with camera-consistent colour is possible
   before enabling it as the default. Compare 1266, 4814, 5791, 5917 at FIT and 1:1.
3. Measure calibration, demosaic, native display and cached reopening separately.
4. Investigate proper camera RGB matrices/white balance, hue-saturation look tables
   or open DCP profiles for Nikon Z 5; do not reintroduce full-frame JPEG pixel fusion.
5. Confirm Edit and Export correctly signal which source they process. The JPEG
   emergency fallback does **not** yet meet the true-RAW export quality objective.

The NEF and user screenshots are deliberately not committed to the public repository.
