'use client';

import { useState, useEffect } from 'react';
import { Plus } from 'lucide-react';
import { getProjects, createProject, type Project } from '@/lib/api';
import { useRouter } from 'next/navigation';

export default function ProjectsPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [newProjectName, setNewProjectName] = useState('');
  const [loading, setLoading] = useState(true);
  const router = useRouter();

  useEffect(() => {
    loadProjects();
  }, []);

  const loadProjects = async () => {
    try {
      const data = await getProjects();
      setProjects(data);
    } catch (error) {
      console.error('Failed to load projects:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleCreateProject = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newProjectName.trim()) return;

    try {
      const project = await createProject(newProjectName);
      setProjects([...projects, project]);
      setNewProjectName('');
    } catch (error) {
      console.error('Failed to create project:', error);
    }
  };

  const handleSelectProject = (projectId: string) => {
    router.push(`/projects/${projectId}`);
  };

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="text-gray-600">Loading projects...</div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50 p-8">
      <div className="max-w-4xl mx-auto">
        <h1 className="text-3xl font-bold text-gray-900 mb-8">Projects</h1>

        {/* Create Project Form */}
        <form onSubmit={handleCreateProject} className="mb-8 flex gap-4">
          <input
            type="text"
            value={newProjectName}
            onChange={(e) => setNewProjectName(e.target.value)}
            placeholder="New project name"
            className="flex-1 px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
          <button
            type="submit"
            className="px-6 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 flex items-center gap-2"
          >
            <Plus size={20} />
            Create Project
          </button>
        </form>

        {/* Projects List */}
        <div className="grid gap-4">
          {projects.length === 0 ? (
            <div className="text-center text-gray-500 py-12">
              No projects yet. Create your first project to get started.
            </div>
          ) : (
            projects.map((project) => (
              <div
                key={project.id}
                onClick={() => handleSelectProject(project.id)}
                className="bg-white p-6 rounded-lg shadow-sm hover:shadow-md cursor-pointer transition-shadow"
              >
                <h2 className="text-xl font-semibold text-gray-900">{project.name}</h2>
                <p className="text-sm text-gray-500 mt-2">
                  Created: {new Date(project.created_at).toLocaleDateString()}
                </p>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
