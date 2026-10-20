"""Generate the original PDFLote document/print-stack icon, no external assets."""
from pathlib import Path
from PIL import Image, ImageDraw

base=Path(__file__).resolve().parent/'assets'
base.mkdir(exist_ok=True)
image=Image.new('RGBA',(512,512),(0,0,0,0))
d=ImageDraw.Draw(image)
d.rounded_rectangle((16,16,496,496),radius=100,fill='#102b46')
d.rounded_rectangle((111,133,385,415),radius=18,fill='#4d7893')
d.rounded_rectangle((95,112,369,394),radius=18,fill='#a5bbc9')
d.rounded_rectangle((79,76,353,363),radius=16,fill='#f5f8fa')
d.polygon([(285,76),(353,144),(285,144)],fill='#8eabbf')
for y,w in [(182,192),(219,168),(256,145)]:
    d.rounded_rectangle((119,y,119+w,y+14),radius=6,fill='#33566f')
d.rounded_rectangle((225,301,438,433),radius=22,fill='#d99242')
d.rectangle((258,284,403,323),fill='#f5f8fa')
d.rounded_rectangle((258,368,403,451),radius=6,fill='#f5f8fa')
for y in (389,410):d.rectangle((278,y,380,y+7),fill='#33566f')
d.ellipse((405,326,418,339),fill='#102b46')
image.save(base/'pdflote.png')
image.save(base/'pdflote.ico',sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
print(base/'pdflote.ico')
