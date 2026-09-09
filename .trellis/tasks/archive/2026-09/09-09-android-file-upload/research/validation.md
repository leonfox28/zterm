# Validation

Native operation, Repository URI staging, OS picker contracts and Compose progress UI are implemented. Emulator native and MediaStore URI transfers are byte-exact; UI dialog instrumentation passed. Physical phone, actual picker navigation and Activity recreation remain manual acceptance.

See [parent evidence](../../09-09-remote-file-upload/research/validation.md) for
commands, assertion points, final quality checks and explicit runtime boundaries.

## Two-button / Back follow-up

The first photo button and second paperclip now open their system pickers directly.
Both real picker Back paths passed twice each on the emulator; see
[two-button evidence](two-buttons-and-picker-back.md). Activity recreation and
physical phone/IME acceptance remain distinct pending evidence.

The user later clarified that the failing input was the emulator sidebar Back
button. The four successful cases above used Android KEYCODE_BACK injection and
did not validate that hardware route. The preserved task stack was healthy.
The preview AVD had hw.keyboard=no; enabling it and cold booting exposes the
missing keyboard with KEY_BACK support. The user subsequently confirmed that the
actual sidebar button now works; the guest event record contains KEY_BACK DOWN/UP
pairs on the restored keyboard device. See the reopened report in the same
evidence note. No APK change was needed; the temporary event recorder was stopped.

## Direction vector follow-up

After user approval, the four text arrows were replaced with 18 dp LineIcon
vectors matching the other toolbar icons, with localized accessible labels.
Android lint/debug build/test compilation and diff checks pass. The updated APK
was installed and visually inspected on emulator-5558; all eleven controls fit.
Current debug APK SHA-256:
`ed02f37f2559ecd7a96d997085c34a601ce3e650ce46d61f1bfaf2b52847052d`.
See the direction-icon section in [UI evidence](two-buttons-and-picker-back.md).
