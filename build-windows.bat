@echo off
rem Windows 10/11 x64, Python 3.11 x64
py -3.11 -m venv .venv
call .venv\Scripts\activate.bat
python -m pip install -r requirements.txt
python -m unittest discover -v
if errorlevel 1 exit /b 1
python -m PyInstaller --noconfirm --clean --onefile --windowed --name PDFLote --icon "assets/pdflote.ico" --add-data "assets;assets" --add-binary "runtime/*.dll;." --collect-all pymupdf --hidden-import win32print --hidden-import win32gui --hidden-import win32timezone --hidden-import PIL.ImageWin --exclude-module numpy --exclude-module onnxruntime --exclude-module networkx --exclude-module pymupdf4llm --exclude-module pymupdf_layout app.py
