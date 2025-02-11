import os
import shutil
import json


script_dir = os.path.dirname(os.path.abspath(__file__))
project_root = os.path.dirname(script_dir)
source_dir = os.path.join(project_root, "node_modules", "highlight.js", "styles")
light_dest_dir = os.path.join(project_root, "public", "highlight.js", "light")
dark_dest_dir = os.path.join(project_root, "public", "highlight.js", "dark")

# Create destination directories if they don't exist
os.makedirs(light_dest_dir, exist_ok=True)
os.makedirs(dark_dest_dir, exist_ok=True)

light_styles = ["ros-pine-dawn", "synth-midnight-terminal-light", "tokyo-night-light"]
dark_styles = [
    "3024",
    "agate",
    "an-old-hope",
    "androidstudio",
    "apathy",
    "apprentice",
    "arta",
    "ashes",
    "atelier-cave",
    "atelier-dune",
    "atelier-estuary",
    "atelier-forest",
    "atelier-heath",
    "atelier-lakeside",
    "atelier-plateau",
    "atelier-savanna",
    "atelier-seaside",
    "atelier-sulphurpool",
    "atlas",
    "bespin",
    "brewer",
    "bright",
    "brogrammer",
    "brown-paper",
    "chalk",
    "circus",
    "codepen-embed",
    "codeschool",
    "colors",
    "danqing",
    "decaf",
    "devibeans",
    "eighties",
    "embers",
    "espresso",
    "eva",
    "eva-dim",
    "far",
    "felipec",
    "flat",
    "framer",
    "gigavolt",
    "gml",
    "green-screen",
    "hardcore",
    "helios",
    "hopscotch",
    "hybrid",
    "isotope",
    "kimber",
    "lioshi",
    "london-tube",
    "macintosh",
    "marrakesh",
    "materia",
    "mellow-purple",
    "mocha",
    "nebula",
    "nova",
    "obsidian",
    "ocean",
    "oceanicnext",
    "paraiso",
    "pasque",
    "phd",
    "pico",
    "pojoaque",
    "pop",
    "porple",
    "qualia",
    "railscasts",
    "rainbow",
    "rebecca",
    "sandcastle",
    "seti-ui",
    "shades-of-purple",
    "snazzy",
    "solar-flare",
    "spacemacs",
    "srcery",
    "summercamp",
    "sunburst",
    "tango",
    "tender",
    "twilight",
    "vs2015",
    "vulcan",
    "windows-10",
    "windows-95",
    "windows-high-contrast",
    "windows-nt",
    "woodland",
    "xt256",
]
ignore_prefix = "black-metal"
ignore_name = [
    "nncs-light",
    "windows-nt-light",
    "windows-high-contrast-light",
    "windows-95-light",
    "windows-10-light",
]


def is_dark_style(filename):
    return (
        "dark" in filename
        or "black" in filename
        or "night" in filename
        or "dusk" in filename
        or "palenight" in filename
        or "monokai" in filename
        or "dracula" in filename
        or "nord" in filename
        or ("material" in filename and "light" not in filename)
        or "ros-pine" in filename
        or "zenburn" in filename
        or "darcula" in filename
    ) and filename.endswith(".min.css")


def run_copy_style():
    light_files = []
    dark_files = []
    # Iterate through files in the source directory
    for filename in os.listdir(source_dir):
        if ignore_prefix in filename:
            continue
        new_filename = filename.replace(".min.css", "")
        if new_filename in ignore_name:
            continue
        if (
            is_dark_style(filename) and new_filename not in light_styles
        ) or new_filename in dark_styles:
            # Copy dark styles to the dark directory
            shutil.copy(
                os.path.join(source_dir, filename),
                os.path.join(dark_dest_dir, new_filename + ".css"),
            )
            dark_files.append(new_filename)
        elif filename.endswith(".min.css"):
            # Copy light styles to the light directory
            shutil.copy(
                os.path.join(source_dir, filename),
                os.path.join(light_dest_dir, new_filename + ".css"),
            )
            light_files.append(new_filename)

    # Check for base16 directory and copy its CSS files
    base16_dir = os.path.join(source_dir, "base16")
    if os.path.exists(base16_dir):
        for filename in os.listdir(base16_dir):
            if ignore_prefix in filename:
                continue
            new_filename = filename.replace(".min.css", "")
            if new_filename in ignore_name:
                continue
            if (
                is_dark_style(filename) and new_filename not in light_styles
            ) or new_filename in dark_styles:
                # Copy dark styles to the dark directory
                shutil.copy(
                    os.path.join(base16_dir, filename),
                    os.path.join(dark_dest_dir, new_filename + ".css"),
                )
                dark_files.append(new_filename)
            elif filename.endswith(".min.css"):
                # Copy light styles to the light directory
                shutil.copy(
                    os.path.join(base16_dir, filename),
                    os.path.join(light_dest_dir, new_filename + ".css"),
                )
                light_files.append(new_filename)

    # generate the json file
    os.makedirs("./src/config/highlight.js", exist_ok=True)
    with open("./src/config/highlight.js/themes.json", "w") as f:
        json.dump(
            {
                "light": sorted(list(set(light_files))),
                "dark": sorted(list(set(dark_files))),
            },
            f,
        )

    print("Files copied successfully.")


if __name__ == "__main__":
    run_copy_style()
