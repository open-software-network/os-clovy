using System;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows.Forms;

[assembly: System.Reflection.AssemblyTitle("Clovy Computer Use Synthetic Fixture")]
[assembly: System.Reflection.AssemblyProduct("Clovy Computer Use Synthetic Fixture")]
[assembly: System.Reflection.AssemblyDescription("Clovy Computer Use Synthetic Fixture")]
[assembly: System.Reflection.AssemblyCompany("Clovy")]
[assembly: System.Reflection.AssemblyVersion("1.0.0.0")]
[assembly: System.Reflection.AssemblyFileVersion("1.0.0.0")]

// Disposable signed QA application. It never reads user files or credentials.
internal static class Fixture {
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
    static string Arg(string[] args, string key) {
        int index = Array.IndexOf(args, key);
        return index >= 0 && index + 1 < args.Length ? args[index + 1] : null;
    }
    static void Write(string path, string value) {
        if (path != null) File.WriteAllText(path, value);
    }
    [STAThread] static int Main(string[] args) {
        SetProcessDPIAware();
        if (Array.IndexOf(args, "--probe") >= 0) {
            uint pid; GetWindowThreadProcessId(GetForegroundWindow(), out pid);
            Point point; if (!GetCursorPos(out point)) return 1;
            Console.WriteLine("{\"frontmostPid\":" + pid + ",\"cursorX\":" + point.X + ",\"cursorY\":" + point.Y + "}");
            return 0;
        }
        string activate = Arg(args, "--activate-pid");
        if (activate != null) return SetForegroundWindow(Process.GetProcessById(int.Parse(activate)).MainWindowHandle) ? 0 : 1;
        bool observer = Arg(args, "--role") == "observer";
        string state = Arg(args, "--state");
        string pidPath = Arg(args, "--pid-file");
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        var window = new Form { Text = observer ? "Clovy Computer Use Observer" : "Clovy Computer Use Target", ClientSize = new Size(430, 230), StartPosition = FormStartPosition.Manual, Location = observer ? new Point(100, 100) : new Point(600, 180) };
        var label = new Label { Text = observer ? "Keep this window in front" : "Synthetic Computer use target", Location = new Point(25, 20), AutoSize = true };
        window.Controls.Add(label);
        if (!observer) {
            var button = new Button { Text = "Apply fixture action", AccessibleName = "Apply fixture action", Location = new Point(25, 60), Size = new Size(200, 40) };
            button.Click += delegate { Write(state, "clicked"); };
            window.Controls.Add(button);
            var input = new TextBox { AccessibleName = "Fixture text input", Location = new Point(25, 120), Size = new Size(260, 30) };
            input.TextChanged += delegate { Write(state, "typed:" + input.Text); };
            window.Controls.Add(input);
            var sensitive = new TextBox { AccessibleName = "Fixture password input", UseSystemPasswordChar = true, Location = new Point(25, 170), Size = new Size(260, 30) };
            window.Controls.Add(sensitive);
        }
        window.Shown += delegate { Write(pidPath, Process.GetCurrentProcess().Id.ToString()); };
        Application.Run(window);
        return 0;
    }
}
