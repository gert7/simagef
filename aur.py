import subprocess
import toml

with open("Cargo.toml", "r") as file:
    original = file.read()

t = toml.load("Cargo.toml")

t['dependencies']['image']['features'] = ["avif-native"]

with open("Cargo.toml", "w") as file:
    toml.dump(t, file)

subprocess.run(["cargo", "aur"])

with open("Cargo.toml", "w") as file:
    file.write(original)

