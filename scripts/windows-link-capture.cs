using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading.Tasks;

internal static class PaperroomLinkCapture
{
    private static string Quote(string argument)
    {
        // Windows argv quoting. Escape backslashes before quotes and the closing quote.
        var result = new StringBuilder("\"");
        int backslashes = 0;
        foreach (char c in argument)
        {
            if (c == '\\')
            {
                backslashes++;
                continue;
            }
            result.Append('\\', c == '"' ? (2 * backslashes) + 1 : backslashes);
            result.Append(c);
            backslashes = 0;
        }
        return result.Append('\\', 2 * backslashes).Append('"').ToString();
    }

    private static int Main(string[] args)
    {
        string linker = Environment.GetEnvironmentVariable("PAPERROOM_REAL_LINK");
        string directory = Environment.GetEnvironmentVariable("PAPERROOM_LINK_TRACE");
        if (String.IsNullOrEmpty(linker) || !Path.IsPathRooted(linker) ||
            !String.Equals(Path.GetFileName(linker), "link.exe", StringComparison.OrdinalIgnoreCase) ||
            !File.Exists(linker) || String.IsNullOrEmpty(directory) ||
            !Path.IsPathRooted(directory) || !Directory.Exists(directory) || args.Length == 0)
        {
            Console.Error.WriteLine("The private MSVC capture environment is invalid.");
            return 125;
        }

        try
        {
            var start = new ProcessStartInfo(linker, String.Join(" ", Array.ConvertAll(args, Quote)))
            {
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true
            };
            using (var output = new FileStream(Path.Combine(directory, "link.stdout"), FileMode.CreateNew))
            using (var errors = new FileStream(Path.Combine(directory, "link.stderr"), FileMode.CreateNew))
            using (var child = Process.Start(start))
            {
                Task stdout = child.StandardOutput.BaseStream.CopyToAsync(output);
                Task stderr = child.StandardError.BaseStream.CopyToAsync(errors);
                Task.WaitAll(stdout, stderr);
                child.WaitForExit();
                return child.ExitCode;
            }
        }
        catch (Exception failure)
        {
            Console.Error.WriteLine("Private MSVC capture failed (" + failure.GetType().Name + ").");
            return 125;
        }
    }
}
