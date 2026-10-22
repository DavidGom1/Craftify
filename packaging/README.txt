Craftify for Windows (64-bit)
=============================

Converts SolidWorks assemblies and parts (.SLDASM, .SLDPRT) and Fusion designs (.f3d, .f3z)
into SolidCraft designs, and checks every part against the original. Neither SolidWorks nor
Fusion is needed.

Before you start
  Nothing to install: SolidCraft's solidcraft-cli, which builds the designs, is in the
  solidcraft folder. To open the designs you need SolidCraft
  (https://github.com/tangency-labs/solidcraft).

Use
  1. Unzip this folder anywhere and keep the files together (Craftify.exe needs cadmpeg.exe
     and the solidcraft folder next to it).
  2. Run Craftify.exe. Windows may warn that the program is not signed: choose
     "More info" > "Run anyway".
  3. Choose a file, an output folder, and press "Convertir".

Command line
  craftify-cli.exe "C:\path\ASSEMBLY.SLDASM" --out "C:\path\output"
  craftify-cli.exe --help

Source code, issues and new versions: https://github.com/DavidGom1/Craftify
Third-party notices: NOTICE.md
