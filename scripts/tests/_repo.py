import pathlib
import tomllib


REPOSITORY = pathlib.Path(__file__).resolve().parents[2]


def manifest(relative_path):
    return tomllib.loads((REPOSITORY / relative_path).read_text(encoding="utf-8"))


def dependency_declarations(cargo_manifest, dependency_name):
    declarations = []
    dependency_sections = ("dependencies", "dev-dependencies", "build-dependencies")
    for section in dependency_sections:
        if dependency_name in cargo_manifest.get(section, {}):
            declarations.append(cargo_manifest[section][dependency_name])
    for target in cargo_manifest.get("target", {}).values():
        for section in dependency_sections:
            if dependency_name in target.get(section, {}):
                declarations.append(target[section][dependency_name])
    return declarations
