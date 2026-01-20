import pyautogui
import time

print("Mouse wiggler started. Press Ctrl+C to stop.")

try:
    while True:
        # Get current mouse position
        x, y = pyautogui.position()
        
        # Wiggle the mouse
        pyautogui.moveRel(10, 0, duration=0.1)
        pyautogui.moveRel(-10, 0, duration=0.1)
        
        print(f"Wiggled at position ({x}, {y})")
        
        # Wait 5 seconds
        time.sleep(5)
        
except KeyboardInterrupt:
    print("\nMouse wiggler stopped.")
