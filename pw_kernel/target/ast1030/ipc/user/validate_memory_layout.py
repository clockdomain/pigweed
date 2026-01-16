#!/usr/bin/env python3
# Copyright 2025 The Pigweed Authors
#
# Licensed under the Apache License, Version 2.0 (the "License"); you may not
# use this file except in compliance with the License. You may obtain a copy of
# the License at
#
#     https://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
# WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
# License for the specific language governing permissions and limitations under
# the License.
"""Validate AST1030 system.json5 memory layout for PMSAv7 compatibility.

This script parses the system.json5 configuration and validates:
1. All memory regions fit within the 640KB AST1030 SRAM
2. No memory regions overlap
3. Apps are placed correctly (flash after kernel flash, RAM after kernel RAM)
4. PMSAv7 subregions don't cause code regions to overlap with kernel RAM
5. Suggests PMSAv7-friendly layouts when issues are found

PMSAv7 Constraints:
- Region sizes must be power-of-2 (32 bytes to 4GB)
- Region base must be aligned to region size
- 8 subregions per region (each 1/8 of total size)
- Subregions can be individually disabled via SRD mask

The key insight is that placing memory regions at power-of-2 aligned addresses
minimizes MPU region bloat and prevents subregion overlap with kernel memory.
"""

import json
import re
import sys
from pathlib import Path


# AST1030 has 768KB SRAM (0x00000000 - 0x000BFFFF)
# But practical limit depends on what's mapped
AST1030_SRAM_SIZE = 768 * 1024  # 0xC0000 = 786432 bytes


def strip_json5_comments(text: str) -> str:
    """Remove // comments from JSON5 to make it valid JSON."""
    lines = []
    for line in text.split('\n'):
        # Remove // comments (but not inside strings)
        # Simple approach: find // not inside quotes
        in_string = False
        result = []
        i = 0
        while i < len(line):
            if line[i] == '"' and (i == 0 or line[i-1] != '\\'):
                in_string = not in_string
                result.append(line[i])
            elif not in_string and line[i:i+2] == '//':
                break  # Rest of line is comment
            else:
                result.append(line[i])
            i += 1
        lines.append(''.join(result))
    return '\n'.join(lines)


def hex_to_decimal(match: re.Match) -> str:
    """Convert hex number to decimal for JSON parsing."""
    return str(int(match.group(0), 16))


def parse_json5(filepath: Path) -> dict:
    """Parse JSON5 file (with comments and trailing commas)."""
    text = filepath.read_text()

    # Strip comments
    text = strip_json5_comments(text)

    # Remove trailing commas before } or ]
    text = re.sub(r',(\s*[}\]])', r'\1', text)

    # Quote unquoted property names (JSON5 -> JSON conversion)
    # Match: word characters followed by colon (property names)
    text = re.sub(r'(\s)([a-zA-Z_][a-zA-Z0-9_]*)(\s*:)', r'\1"\2"\3', text)

    # Convert hex numbers (0x...) to decimal (JSON doesn't support hex)
    text = re.sub(r'0x[0-9a-fA-F]+', hex_to_decimal, text)

    return json.loads(text)


def format_hex(value: int) -> str:
    """Format integer as hex string."""
    return f"0x{value:08X}"


def format_size(size: int) -> str:
    """Format size in human-readable form."""
    if size >= 1024 * 1024:
        return f"{size // (1024 * 1024)}MB"
    elif size >= 1024:
        return f"{size // 1024}KB"
    return f"{size} bytes"


def is_power_of_2(n: int) -> bool:
    """Check if n is a power of 2."""
    return n > 0 and (n & (n - 1)) == 0


def next_power_of_2(n: int) -> int:
    """Return the smallest power of 2 >= n."""
    if n <= 0:
        return 1
    if is_power_of_2(n):
        return n
    return 1 << n.bit_length()


def align_up(value: int, alignment: int) -> int:
    """Align value up to the next multiple of alignment."""
    return (value + alignment - 1) & ~(alignment - 1)


def align_down(value: int, alignment: int) -> int:
    """Align value down to the previous multiple of alignment."""
    return value & ~(alignment - 1)


def check_pmsav7_friendliness(start: int, size: int) -> dict:
    """Check how PMSAv7-friendly a memory region placement is.

    Returns a dict with:
    - is_friendly: True if the region will map to a minimal MPU region
    - optimal_start: Suggested start address for optimal placement
    - bloat_factor: How much larger the MPU region is vs requested size
    """
    end = start + size

    # Calculate actual PMSAv7 region
    pmsav7 = calculate_pmsav7_region(start, end)

    # Ideal: region size equals requested size (power-of-2) and base is aligned
    ideal_size = next_power_of_2(size)
    bloat_factor = pmsav7['size'] / size if size > 0 else float('inf')

    # A region is "friendly" if the MPU region is at most 2x the requested size
    # and the start address is power-of-2 aligned
    is_friendly = bloat_factor <= 2.0 and (start & (ideal_size - 1)) == 0

    # Calculate optimal start: align to the size of the region
    optimal_start = align_up(start, ideal_size)

    return {
        'is_friendly': is_friendly,
        'optimal_start': optimal_start,
        'ideal_size': ideal_size,
        'actual_mpu_size': pmsav7['size'],
        'bloat_factor': bloat_factor,
        'start_aligned': (start & (ideal_size - 1)) == 0,
    }


def calculate_pmsav7_region(start: int, end: int) -> dict:
    """Calculate the PMSAv7 aligned region for a memory range.

    PMSAv7 requires:
    - Power-of-2 region sizes (32 bytes to 4GB)
    - Region base aligned to region size
    - 8 subregions per region
    """
    requested_size = end - start

    # Find smallest power-of-2 region size that covers the range
    region_size = 32  # Minimum 32 bytes
    while region_size < requested_size:
        region_size *= 2

    # Align base to region size
    aligned_base = start & ~(region_size - 1)

    # Check if aligned region covers the end address
    while aligned_base + region_size < end:
        region_size *= 2
        aligned_base = start & ~(region_size - 1)

    # Calculate SIZE field: log2(region_size) - 1
    size_field = region_size.bit_length() - 2

    # Calculate subregion size and which are enabled
    subregion_size = region_size // 8
    enabled_subregions = []
    srd_mask = 0

    for i in range(8):
        sr_start = aligned_base + i * subregion_size
        sr_end = sr_start + subregion_size
        # Subregion overlaps if: sr_start < end AND sr_end > start
        if sr_start < end and sr_end > start:
            enabled_subregions.append(i)
        else:
            srd_mask |= (1 << i)

    return {
        'base': aligned_base,
        'size': region_size,
        'size_field': size_field,
        'subregion_size': subregion_size,
        'enabled_subregions': enabled_subregions,
        'srd_mask': srd_mask,
    }


def check_pmsav7_subregion_overlap(code_region: dict, pmsav7: dict,
                                    kernel_ram: dict) -> list[str]:
    """Check if PMSAv7 subregions of a code region overlap with kernel RAM."""
    errors = []

    for sr in pmsav7['enabled_subregions']:
        sr_start = pmsav7['base'] + sr * pmsav7['subregion_size']
        sr_end = sr_start + pmsav7['subregion_size']

        # Check if this subregion overlaps with kernel RAM
        if sr_start < kernel_ram['end'] and sr_end > kernel_ram['start']:
            overlap_start = max(sr_start, kernel_ram['start'])
            overlap_end = min(sr_end, kernel_ram['end'])

            errors.append(
                f"PMSAv7 SUBREGION OVERLAP: {code_region['name']} subregion {sr} "
                f"[{format_hex(sr_start)}-{format_hex(sr_end)}] overlaps with "
                f"Kernel RAM [{format_hex(kernel_ram['start'])}-{format_hex(kernel_ram['end'])}] "
                f"at [{format_hex(overlap_start)}-{format_hex(overlap_end)}]"
            )

    return errors


def suggest_pmsav7_friendly_layout(config: dict) -> None:
    """Suggest a PMSAv7-friendly memory layout based on the current config.

    The system generator places:
    - App flash immediately after kernel flash
    - App RAM immediately after kernel RAM

    For PMSAv7 compatibility, we need:
    - Kernel flash to end at a power-of-2 boundary (so app flash starts aligned)
    - Kernel RAM to start AFTER all app flash ends
    - All regions to be power-of-2 sized and aligned
    """
    print("\n" + "=" * 70)
    print("SUGGESTED PMSAv7-FRIENDLY LAYOUT")
    print("=" * 70)

    arch = config.get('arch', {})
    kernel = config.get('kernel', {})
    apps_config = config.get('apps', {})

    # Convert apps to list format
    if isinstance(apps_config, dict):
        apps = [{'name': k, **v} for k, v in apps_config.items()]
    else:
        apps = apps_config

    vector_table_size = arch.get('vector_table_size_bytes', 0x420)

    print("\nThe system generator places app flash after kernel flash, and")
    print("app RAM after kernel RAM. For PMSAv7, we need all regions at")
    print("power-of-2 aligned addresses to avoid MPU subregion overlap.\n")

    print("Strategy: Kernel flash ends at power-of-2 boundary, so apps are aligned.")
    print("Kernel RAM starts after all flash regions end.\n")

    # Calculate total app flash needed
    total_app_flash = sum(app.get('flash_size_bytes', 0) for app in apps)
    total_app_ram = sum(app.get('ram_size_bytes', 0) for app in apps)

    # Find optimal kernel flash size that ends at a power-of-2 boundary
    # We want kernel_flash_start + kernel_flash_size to be power-of-2 aligned
    # where alignment is at least the largest app flash size
    max_app_flash = max(app.get('flash_size_bytes', 0) for app in apps) if apps else 0
    app_flash_alignment = next_power_of_2(max_app_flash)

    # Kernel flash starts at vector_table_size (0x420)
    # We need it to end at an address aligned to app_flash_alignment
    # So: kernel_flash_end = align_up(vector_table_size + min_kernel_flash, app_flash_alignment)
    min_kernel_flash = 64 * 1024  # Minimum 64KB for kernel code

    # Round kernel flash end up to alignment
    kernel_flash_end = align_up(vector_table_size + min_kernel_flash, app_flash_alignment)
    kernel_flash_size = kernel_flash_end - vector_table_size

    print(f"Vector table:    {format_hex(0)} - {format_hex(vector_table_size)}")
    print(f"Kernel code:     {format_hex(vector_table_size)} - {format_hex(kernel_flash_end)}")
    print(f"  Size: {format_size(kernel_flash_size)}")
    print()

    # Place app flash regions at power-of-2 aligned addresses
    app_flash_start = kernel_flash_end
    print("App code regions (placed by system generator after kernel flash):")

    app_flash_regions = []
    for app in apps:
        app_name = app.get('name', 'unknown')
        flash_size = app.get('flash_size_bytes', 0)
        flash_size_p2 = next_power_of_2(flash_size)

        # System generator aligns to 4 bytes, but we want power-of-2 for PMSAv7
        app_flash_aligned = align_up(app_flash_start, flash_size_p2)

        app_flash_regions.append({
            'name': app_name,
            'start': app_flash_aligned,
            'end': app_flash_aligned + flash_size_p2,
            'size': flash_size_p2,
        })

        print(f"  {app_name}: {format_hex(app_flash_aligned)} - {format_hex(app_flash_aligned + flash_size_p2)}")
        print(f"    Size: {format_size(flash_size_p2)} (requested {format_size(flash_size)})")

        app_flash_start = app_flash_aligned + flash_size_p2

    # Kernel RAM must start AFTER all flash ends
    all_flash_end = app_flash_start
    print()
    print(f"All flash ends at: {format_hex(all_flash_end)}")

    # For PMSAv7, kernel RAM should be power-of-2 sized and aligned
    # Use a reasonable size that fits the remaining memory
    remaining_memory = AST1030_SRAM_SIZE - all_flash_end - total_app_ram
    kernel_ram_size = min(256 * 1024, remaining_memory)  # Cap at 256KB
    kernel_ram_size_p2 = next_power_of_2(kernel_ram_size) // 2  # Round down to fit
    if kernel_ram_size_p2 < 32 * 1024:
        kernel_ram_size_p2 = 32 * 1024  # Minimum 32KB

    kernel_ram_start = align_up(all_flash_end, kernel_ram_size_p2)
    kernel_ram_end = kernel_ram_start + kernel_ram_size_p2

    print()
    print(f"Kernel RAM:      {format_hex(kernel_ram_start)} - {format_hex(kernel_ram_end)}")
    print(f"  Size: {format_size(kernel_ram_size_p2)} (power-of-2)")
    print(f"  Aligned to: {format_size(kernel_ram_size_p2)}")

    # App RAM after kernel RAM
    app_ram_start = kernel_ram_end
    print()
    print("App RAM regions (placed by system generator after kernel RAM):")

    for app in apps:
        app_name = app.get('name', 'unknown')
        ram_size = app.get('ram_size_bytes', 0)
        ram_size_p2 = next_power_of_2(ram_size)

        app_ram_aligned = align_up(app_ram_start, 8)  # System generator uses 8-byte alignment

        print(f"  {app_name}: {format_hex(app_ram_aligned)} - {format_hex(app_ram_aligned + ram_size)}")
        print(f"    Size: {format_size(ram_size)}")

        app_ram_start = app_ram_aligned + ram_size

    total_used = app_ram_start
    print()
    print(f"Total memory used: {format_hex(total_used)} ({format_size(total_used)})")

    if total_used > AST1030_SRAM_SIZE:
        print(f"  WARNING: Exceeds AST1030 SRAM ({format_size(AST1030_SRAM_SIZE)})")
    else:
        print(f"  Fits within AST1030 SRAM ({format_size(AST1030_SRAM_SIZE)})")

    # Verify PMSAv7 friendliness of suggested layout
    print()
    print("-" * 70)
    print("PMSAv7 VERIFICATION OF SUGGESTED LAYOUT:")
    print("-" * 70)

    all_good = True
    for region in app_flash_regions:
        pmsav7 = calculate_pmsav7_region(region['start'], region['end'])
        bloat = pmsav7['size'] / region['size']

        # Check if any enabled subregion overlaps kernel RAM
        overlaps = []
        for sr in pmsav7['enabled_subregions']:
            sr_start = pmsav7['base'] + sr * pmsav7['subregion_size']
            sr_end = sr_start + pmsav7['subregion_size']
            if sr_start < kernel_ram_end and sr_end > kernel_ram_start:
                overlaps.append(sr)

        status = "OK" if not overlaps and bloat <= 2.0 else "PROBLEM"
        if overlaps:
            all_good = False

        print(f"  {region['name']}: MPU region {format_size(pmsav7['size'])} "
              f"(bloat: {bloat:.1f}x) - {status}")
        if overlaps:
            print(f"    Subregions {overlaps} overlap kernel RAM!")

    if all_good:
        print("\n  All app code regions have clean MPU mappings.")

    # Print suggested system.json5 values
    print("\n" + "-" * 70)
    print("SUGGESTED system.json5 VALUES:")
    print("-" * 70)
    print(f"""
kernel: {{
    flash_start_address: {format_hex(vector_table_size)},
    flash_size_bytes: {kernel_flash_size},  // {format_size(kernel_flash_size)} (ends at {format_hex(kernel_flash_end)})
    ram_start_address: {format_hex(kernel_ram_start)},  // After all flash
    ram_size_bytes: {kernel_ram_size_p2},  // {format_size(kernel_ram_size_p2)}
}},

// Apps will be placed:
//   Flash: starting at {format_hex(kernel_flash_end)} (after kernel flash)
//   RAM: starting at {format_hex(kernel_ram_end)} (after kernel RAM)
""")


def validate_memory_layout(config: dict) -> list[str]:
    """Validate memory layout and return list of errors."""
    errors = []
    warnings = []

    # Extract configuration
    arch = config.get('arch', {})
    kernel = config.get('kernel', {})
    apps_config = config.get('apps', {})

    # Handle both list and dict formats for apps
    if isinstance(apps_config, dict):
        apps = [{'name': k, **v} for k, v in apps_config.items()]
    else:
        apps = apps_config

    vector_table_start = arch.get('vector_table_start_address', 0)
    vector_table_size = arch.get('vector_table_size_bytes', 0)

    kernel_flash_start = kernel.get('flash_start_address', 0)
    kernel_flash_size = kernel.get('flash_size_bytes', 0)
    kernel_ram_start = kernel.get('ram_start_address', 0)
    kernel_ram_size = kernel.get('ram_size_bytes', 0)

    # Build list of all memory regions
    regions = []

    # Vector table
    regions.append({
        'name': 'Vector Table',
        'start': vector_table_start,
        'size': vector_table_size,
        'end': vector_table_start + vector_table_size,
        'type': 'flash'
    })

    # Kernel flash
    regions.append({
        'name': 'Kernel Flash',
        'start': kernel_flash_start,
        'size': kernel_flash_size,
        'end': kernel_flash_start + kernel_flash_size,
        'type': 'flash'
    })

    # App flash regions (placed after kernel flash)
    app_flash_start = kernel_flash_start + kernel_flash_size
    for app in apps:
        app_name = app.get('name', 'unknown')
        flash_size = app.get('flash_size_bytes', 0)
        regions.append({
            'name': f"App '{app_name}' Flash",
            'start': app_flash_start,
            'size': flash_size,
            'end': app_flash_start + flash_size,
            'type': 'flash'
        })
        app_flash_start += flash_size

    # Kernel RAM
    regions.append({
        'name': 'Kernel RAM',
        'start': kernel_ram_start,
        'size': kernel_ram_size,
        'end': kernel_ram_start + kernel_ram_size,
        'type': 'ram'
    })

    # App RAM regions (placed after kernel RAM)
    app_ram_start = kernel_ram_start + kernel_ram_size
    for app in apps:
        app_name = app.get('name', 'unknown')
        ram_size = app.get('ram_size_bytes', 0)
        regions.append({
            'name': f"App '{app_name}' RAM",
            'start': app_ram_start,
            'size': ram_size,
            'end': app_ram_start + ram_size,
            'type': 'ram'
        })
        app_ram_start += ram_size

    # Print memory map
    print("=" * 70)
    print("MEMORY LAYOUT")
    print("=" * 70)
    print(f"{'Region':<25} {'Start':>12} {'End':>12} {'Size':>10}")
    print("-" * 70)

    for region in regions:
        print(f"{region['name']:<25} {format_hex(region['start']):>12} "
              f"{format_hex(region['end']):>12} {format_size(region['size']):>10}")

    print("-" * 70)

    # Find highest address used
    max_addr = max(r['end'] for r in regions)
    print(f"{'Total used':<25} {format_hex(0):>12} {format_hex(max_addr):>12} "
          f"{format_size(max_addr):>10}")
    print(f"{'AST1030 SRAM limit':<25} {format_hex(0):>12} "
          f"{format_hex(AST1030_SRAM_SIZE):>12} {format_size(AST1030_SRAM_SIZE):>10}")
    print("=" * 70)

    # Validation 1: Check all regions fit within SRAM
    for region in regions:
        if region['end'] > AST1030_SRAM_SIZE:
            errors.append(
                f"Region '{region['name']}' exceeds SRAM limit: "
                f"ends at {format_hex(region['end'])} but limit is {format_hex(AST1030_SRAM_SIZE)}"
            )

    # Validation 2: Check for overlapping regions
    for i, r1 in enumerate(regions):
        for r2 in regions[i+1:]:
            # Check if regions overlap
            if r1['start'] < r2['end'] and r2['start'] < r1['end']:
                errors.append(
                    f"OVERLAP: '{r1['name']}' [{format_hex(r1['start'])}-{format_hex(r1['end'])}] "
                    f"overlaps with '{r2['name']}' [{format_hex(r2['start'])}-{format_hex(r2['end'])}]"
                )

    # Validation 3: Check flash regions are contiguous and before RAM
    flash_regions = [r for r in regions if r['type'] == 'flash']
    ram_regions = [r for r in regions if r['type'] == 'ram']

    if flash_regions and ram_regions:
        max_flash_end = max(r['end'] for r in flash_regions)
        min_ram_start = min(r['start'] for r in ram_regions)

        if max_flash_end > min_ram_start:
            errors.append(
                f"Flash regions extend past RAM start: "
                f"flash ends at {format_hex(max_flash_end)} but RAM starts at {format_hex(min_ram_start)}"
            )

    # Validation 4: Check vector table is at address 0
    if vector_table_start != 0:
        warnings.append(
            f"Vector table should start at 0x0, but starts at {format_hex(vector_table_start)}"
        )

    # Validation 5: Check kernel flash starts after vector table
    expected_kernel_start = vector_table_start + vector_table_size
    if kernel_flash_start != expected_kernel_start:
        warnings.append(
            f"Kernel flash should start at {format_hex(expected_kernel_start)} "
            f"(after vector table), but starts at {format_hex(kernel_flash_start)}"
        )

    # Validation 6: PMSAv7 subregion overlap check
    print("\n" + "=" * 70)
    print("PMSAv7 SUBREGION ANALYSIS")
    print("=" * 70)

    kernel_ram_region = next(r for r in regions if r['name'] == 'Kernel RAM')
    code_regions = [r for r in regions if 'Flash' in r['name']]

    for code_region in code_regions:
        pmsav7 = calculate_pmsav7_region(code_region['start'], code_region['end'])

        print(f"\n{code_region['name']} [{format_hex(code_region['start'])}-{format_hex(code_region['end'])}]:")
        print(f"  PMSAv7 region: base={format_hex(pmsav7['base'])}, "
              f"size={format_size(pmsav7['size'])}, SIZE_FIELD={pmsav7['size_field']}")
        print(f"  Subregion size: {format_size(pmsav7['subregion_size'])}")
        print(f"  SRD mask: 0x{pmsav7['srd_mask']:02X} = 0b{pmsav7['srd_mask']:08b}")
        print(f"  Enabled subregions: {pmsav7['enabled_subregions']}")

        # Show subregion details
        for sr in range(8):
            sr_start = pmsav7['base'] + sr * pmsav7['subregion_size']
            sr_end = sr_start + pmsav7['subregion_size']
            status = "ENABLED" if sr in pmsav7['enabled_subregions'] else "disabled"
            print(f"    SR{sr}: {format_hex(sr_start)}-{format_hex(sr_end)} [{status}]")

        # Check for overlap with kernel RAM
        overlap_errors = check_pmsav7_subregion_overlap(
            code_region, pmsav7, kernel_ram_region)
        errors.extend(overlap_errors)

        if overlap_errors:
            for err in overlap_errors:
                print(f"    ❌ {err}")

    if not any('SUBREGION OVERLAP' in e for e in errors):
        print("\n✅ No PMSAv7 subregion overlaps with Kernel RAM")

    # Print warnings
    if warnings:
        print("\nWARNINGS:")
        for w in warnings:
            print(f"  ⚠️  {w}")

    # Print errors
    if errors:
        print("\nERRORS:")
        for e in errors:
            print(f"  ❌ {e}")

    return errors


def main():
    import argparse

    parser = argparse.ArgumentParser(
        description="Validate AST1030 memory layout for PMSAv7 compatibility"
    )
    parser.add_argument(
        '--suggest', '-s',
        action='store_true',
        help='Show suggested PMSAv7-friendly layout even if validation passes'
    )
    parser.add_argument(
        'config_file',
        nargs='?',
        type=Path,
        help='Path to system.json5 (default: same directory as script)'
    )
    args = parser.parse_args()

    # Find system.json5
    if args.config_file:
        config_file = args.config_file
    else:
        script_dir = Path(__file__).parent
        config_file = script_dir / "system.json5"

    if not config_file.exists():
        print(f"Error: {config_file} not found")
        sys.exit(1)

    print(f"Validating: {config_file}\n")

    try:
        config = parse_json5(config_file)
    except json.JSONDecodeError as e:
        print(f"Error parsing JSON5: {e}")
        sys.exit(1)

    errors = validate_memory_layout(config)

    # Show suggestion if there are PMSAv7 overlap errors or if --suggest flag
    has_overlap_errors = any('SUBREGION OVERLAP' in e for e in errors)
    if has_overlap_errors or args.suggest:
        suggest_pmsav7_friendly_layout(config)

    if errors:
        print(f"\n❌ VALIDATION FAILED: {len(errors)} error(s) found")
        sys.exit(1)
    else:
        print("\n✅ VALIDATION PASSED: Memory layout is valid")
        sys.exit(0)


if __name__ == "__main__":
    main()
